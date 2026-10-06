//! Pagination for CGI requests.
//!
//! A [`Paged`] request wraps a [`CgiRequest`] with a [`PagerStrategy`] that
//! derives the parameters of the next page from the previous response. It
//! mirrors the strategies of the upstream library:
//!
//! * [`PageStrategy`] – page numbers;
//! * [`OffsetStrategy`] – offsets;
//! * [`CursorStrategy`] – opaque cursors (also covers "batch refresh");
//! * [`FnStrategy`] – arbitrary closures (multi-field continuation).
//!
//! ```no_run
//! # async fn demo(client: qqmusic_api::Client) -> qqmusic_api::Result<()> {
//! use futures::StreamExt;
//! let paged = client.song().get_related_songlist(97773, vec![]);
//! let first = paged.clone().await?;            // first page only
//! let songs = paged.clone().collect_items(Some(60)).await?; // up to 60 items
//! let mut pages = paged.stream(Some(3));         // up to 3 pages
//! while let Some(page) = pages.next().await {
//!     let _page = page?;
//! }
//! # let _ = (first, songs); Ok(()) }
//! ```

use std::future::IntoFuture;
use std::sync::Arc;

use futures::StreamExt;
use futures::stream::BoxStream;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::json::FromJson;
use crate::request::{BoxFuture, CgiRequest};

/// Extracts a value from a page response.
pub type Extractor<T, X> = Arc<dyn Fn(&T) -> Option<X> + Send + Sync>;

/// Computes the parameters of the next page.
pub trait PagerStrategy<T>: Send + Sync + 'static {
    /// Return the parameters of the next page or `None` when exhausted.
    fn next_params(&self, params: &Value, response: &T) -> Result<Option<Value>>;
}

/// Responses that contain a list of items.
pub trait PageItems {
    /// Item type.
    type Item;
    /// Take the items of the page.
    fn into_items(self) -> Vec<Self::Item>;
}

fn get_int(params: &Value, key: &str) -> Option<i64> {
    params.get(key).and_then(Value::as_i64)
}

fn with_param(params: &Value, key: &str, value: Value) -> Value {
    let mut params = params.clone();
    if !params.is_object() {
        params = Value::Object(serde_json::Map::new());
    }
    if let Some(map) = params.as_object_mut() {
        map.insert(key.to_string(), value);
    }
    params
}

/// Page number based pagination.
pub struct PageStrategy<T> {
    /// Parameter holding the page number.
    pub page_key: String,
    /// Number of the first page.
    pub start_page: i64,
    /// Items per page.
    pub page_size: Option<i64>,
    /// Explicit "has more" flag.
    pub has_more: Option<Extractor<T, bool>>,
    /// Total number of items.
    pub total: Option<Extractor<T, i64>>,
    /// Number of items in the page.
    pub count: Option<Extractor<T, i64>>,
}

impl<T> PageStrategy<T> {
    /// Strategy using `page_key`, starting at page 1.
    pub fn new(page_key: impl Into<String>) -> Self {
        Self {
            page_key: page_key.into(),
            start_page: 1,
            page_size: None,
            has_more: None,
            total: None,
            count: None,
        }
    }

    /// First page number.
    #[must_use]
    pub fn start_page(mut self, start: i64) -> Self {
        self.start_page = start;
        self
    }

    /// Items per page.
    #[must_use]
    pub fn page_size(mut self, size: i64) -> Self {
        self.page_size = Some(size);
        self
    }

    /// "Has more" flag extractor.
    #[must_use]
    pub fn has_more(mut self, f: impl Fn(&T) -> Option<bool> + Send + Sync + 'static) -> Self {
        self.has_more = Some(Arc::new(f));
        self
    }

    /// Total extractor.
    #[must_use]
    pub fn total(mut self, f: impl Fn(&T) -> Option<i64> + Send + Sync + 'static) -> Self {
        self.total = Some(Arc::new(f));
        self
    }

    /// Page item count extractor.
    #[must_use]
    pub fn count(mut self, f: impl Fn(&T) -> Option<i64> + Send + Sync + 'static) -> Self {
        self.count = Some(Arc::new(f));
        self
    }

    fn current_page(&self, params: &Value) -> Result<i64> {
        match params.get(&self.page_key) {
            None => Ok(self.start_page),
            Some(value) => value
                .as_i64()
                .ok_or_else(|| Error::invalid_argument("分页请求缺少有效的页码参数, 无法判断是否存在下一页")),
        }
    }

    fn has_next(&self, params: &Value, response: &T) -> Result<bool> {
        if let Some(flag) = self.has_more.as_ref().and_then(|f| f(response)) {
            return Ok(flag);
        }
        if let (Some(total_fn), Some(page_size)) = (&self.total, self.page_size)
            && let Some(total) = total_fn(response)
        {
            let consumed = self.current_page(params)? - self.start_page + 1;
            return Ok(consumed * page_size < total);
        }
        if let Some(count) = self.count.as_ref().and_then(|f| f(response)) {
            return Ok(match self.page_size {
                Some(size) => count >= size && count > 0,
                None => count > 0,
            });
        }
        Ok(false)
    }
}

impl<T: 'static> PagerStrategy<T> for PageStrategy<T> {
    fn next_params(&self, params: &Value, response: &T) -> Result<Option<Value>> {
        if !self.has_next(params, response)? {
            return Ok(None);
        }
        let page = self.current_page(params)?;
        Ok(Some(with_param(params, &self.page_key, Value::from(page + 1))))
    }
}

/// Offset based pagination.
pub struct OffsetStrategy<T> {
    /// Parameter holding the offset.
    pub offset_key: String,
    /// Parameter holding the page size.
    pub page_size_key: Option<String>,
    /// Fixed page size.
    pub page_size: Option<i64>,
    /// Offset of the first page.
    pub start_offset: i64,
    /// Explicit "has more" flag.
    pub has_more: Option<Extractor<T, bool>>,
    /// Total number of items.
    pub total: Option<Extractor<T, i64>>,
    /// Number of items in the page.
    pub count: Option<Extractor<T, i64>>,
}

impl<T> OffsetStrategy<T> {
    /// Offset in `offset_key`, page size read from `page_size_key`.
    pub fn with_size_key(offset_key: impl Into<String>, page_size_key: impl Into<String>) -> Self {
        Self {
            offset_key: offset_key.into(),
            page_size_key: Some(page_size_key.into()),
            page_size: None,
            start_offset: 0,
            has_more: None,
            total: None,
            count: None,
        }
    }

    /// Offset in `offset_key` with a fixed page size.
    pub fn with_size(offset_key: impl Into<String>, page_size: i64) -> Self {
        Self {
            offset_key: offset_key.into(),
            page_size_key: None,
            page_size: Some(page_size),
            start_offset: 0,
            has_more: None,
            total: None,
            count: None,
        }
    }

    /// Offset of the first page.
    #[must_use]
    pub fn start_offset(mut self, start: i64) -> Self {
        self.start_offset = start;
        self
    }

    /// "Has more" flag extractor.
    #[must_use]
    pub fn has_more(mut self, f: impl Fn(&T) -> Option<bool> + Send + Sync + 'static) -> Self {
        self.has_more = Some(Arc::new(f));
        self
    }

    /// Total extractor.
    #[must_use]
    pub fn total(mut self, f: impl Fn(&T) -> Option<i64> + Send + Sync + 'static) -> Self {
        self.total = Some(Arc::new(f));
        self
    }

    /// Page item count extractor.
    #[must_use]
    pub fn count(mut self, f: impl Fn(&T) -> Option<i64> + Send + Sync + 'static) -> Self {
        self.count = Some(Arc::new(f));
        self
    }

    fn page_size_of(&self, params: &Value) -> Result<i64> {
        if let Some(size) = self.page_size {
            return Ok(size);
        }
        self.page_size_key
            .as_deref()
            .and_then(|key| get_int(params, key))
            .ok_or_else(|| Error::invalid_argument("分页请求缺少有效的 page_size 参数, 无法计算下一页偏移量"))
    }

    fn step(&self, params: &Value, response: &T) -> Result<i64> {
        if let Some(count) = self.count.as_ref().and_then(|f| f(response)) {
            return Ok(count);
        }
        self.page_size_of(params)
    }

    fn current_offset(&self, params: &Value) -> Result<i64> {
        match params.get(&self.offset_key) {
            None => Ok(self.start_offset),
            Some(value) => value
                .as_i64()
                .ok_or_else(|| Error::invalid_argument("分页请求缺少有效的 offset 参数, 无法计算下一页")),
        }
    }

    fn has_next(&self, params: &Value, response: &T) -> Result<bool> {
        if let Some(flag) = self.has_more.as_ref().and_then(|f| f(response)) {
            return Ok(flag);
        }
        if let Some(total) = self.total.as_ref().and_then(|f| f(response)) {
            let offset = self.current_offset(params)?;
            let step = self.step(params, response)?;
            if step <= 0 {
                return Ok(false);
            }
            return Ok(offset + step < total);
        }
        if let Some(count) = self.count.as_ref().and_then(|f| f(response)) {
            let size = self.page_size_of(params)?;
            return Ok(count >= size && count > 0);
        }
        Ok(false)
    }
}

impl<T: 'static> PagerStrategy<T> for OffsetStrategy<T> {
    fn next_params(&self, params: &Value, response: &T) -> Result<Option<Value>> {
        if !self.has_next(params, response)? {
            return Ok(None);
        }
        let step = self.step(params, response)?;
        if step <= 0 {
            return Err(Error::invalid_argument("分页响应未提供有效的当前页数量, 无法计算下一页偏移量"));
        }
        let offset = self.current_offset(params)?;
        Ok(Some(with_param(params, &self.offset_key, Value::from(offset + step))))
    }
}

/// Cursor based pagination.
pub struct CursorStrategy<T> {
    /// Parameter holding the cursor.
    pub cursor_key: String,
    /// Extracts the next cursor.
    pub cursor: Extractor<T, Value>,
    /// Explicit "has more" flag.
    pub has_more: Option<Extractor<T, bool>>,
    /// Number of items in the page.
    pub count: Option<Extractor<T, i64>>,
    /// Expected page size.
    pub page_size: Option<i64>,
    /// Continue even when the cursor did not change ("batch refresh").
    pub allow_repeat: bool,
}

impl<T> CursorStrategy<T> {
    /// Strategy writing the extracted cursor to `cursor_key`.
    pub fn new(cursor_key: impl Into<String>, cursor: impl Fn(&T) -> Option<Value> + Send + Sync + 'static) -> Self {
        Self {
            cursor_key: cursor_key.into(),
            cursor: Arc::new(cursor),
            has_more: None,
            count: None,
            page_size: None,
            allow_repeat: false,
        }
    }

    /// "Has more" flag extractor.
    #[must_use]
    pub fn has_more(mut self, f: impl Fn(&T) -> Option<bool> + Send + Sync + 'static) -> Self {
        self.has_more = Some(Arc::new(f));
        self
    }

    /// Page item count extractor.
    #[must_use]
    pub fn count(mut self, f: impl Fn(&T) -> Option<i64> + Send + Sync + 'static) -> Self {
        self.count = Some(Arc::new(f));
        self
    }

    /// Expected page size.
    #[must_use]
    pub fn page_size(mut self, size: i64) -> Self {
        self.page_size = Some(size);
        self
    }

    /// Allow repeated cursors.
    #[must_use]
    pub fn allow_repeat(mut self, allow: bool) -> Self {
        self.allow_repeat = allow;
        self
    }

    fn terminated(&self, response: &T) -> bool {
        if let Some(flag) = self.has_more.as_ref().and_then(|f| f(response)) {
            return !flag;
        }
        if let Some(count) = self.count.as_ref().and_then(|f| f(response))
            && (self.page_size.is_some_and(|size| count < size) || count == 0)
        {
            return true;
        }
        false
    }
}

impl<T: 'static> PagerStrategy<T> for CursorStrategy<T> {
    fn next_params(&self, params: &Value, response: &T) -> Result<Option<Value>> {
        if self.terminated(response) {
            return Ok(None);
        }
        let Some(cursor) = (self.cursor)(response).filter(|c| !c.is_null()) else {
            return Ok(None);
        };
        if !self.allow_repeat && params.get(&self.cursor_key) == Some(&cursor) {
            return Ok(None);
        }
        Ok(Some(with_param(params, &self.cursor_key, cursor)))
    }
}

/// Closure based strategy.
pub struct FnStrategy<F>(pub F);

impl<T, F> PagerStrategy<T> for FnStrategy<F>
where
    F: Fn(&Value, &T) -> Option<Value> + Send + Sync + 'static,
{
    fn next_params(&self, params: &Value, response: &T) -> Result<Option<Value>> {
        Ok((self.0)(params, response))
    }
}

/// A paginated CGI request.
///
/// Awaiting it returns the first page.
#[must_use = "requests do nothing unless awaited"]
pub struct Paged<T> {
    request: CgiRequest<T>,
    strategy: Arc<dyn PagerStrategy<T>>,
}

impl<T> Clone for Paged<T> {
    fn clone(&self) -> Self {
        Self {
            request: self.request.clone(),
            strategy: Arc::clone(&self.strategy),
        }
    }
}

impl<T> std::fmt::Debug for Paged<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Paged").field("request", &self.request).finish_non_exhaustive()
    }
}

impl<T: FromJson + Send + 'static> Paged<T> {
    /// Wrap a request.
    pub fn new(request: CgiRequest<T>, strategy: impl PagerStrategy<T>) -> Self {
        Self {
            request,
            strategy: Arc::new(strategy),
        }
    }

    /// The first page request.
    pub fn request(&self) -> &CgiRequest<T> {
        &self.request
    }

    /// Modify the underlying request (credential, platform, comm, ...).
    pub fn map_request(mut self, f: impl FnOnce(CgiRequest<T>) -> CgiRequest<T>) -> Self {
        self.request = f(self.request);
        self
    }

    /// Fetch the first page.
    pub async fn send(self) -> Result<T> {
        self.request.send().await
    }

    /// Page iterator (`limit` = maximum number of pages).
    pub fn pager(self, limit: Option<usize>) -> Pager<T> {
        Pager {
            next: Some(self.request),
            strategy: self.strategy,
            limit,
            yielded: 0,
        }
    }

    /// Stream of pages.
    pub fn stream(self, limit: Option<usize>) -> BoxStream<'static, Result<T>> {
        futures::stream::unfold(self.pager(limit), |mut pager| async move {
            pager.next_page().await.map(|page| (page, pager))
        })
        .boxed()
    }

    /// Collect pages.
    pub async fn collect(self, limit: Option<usize>) -> Result<Vec<T>> {
        let mut pager = self.pager(limit);
        let mut pages = Vec::new();
        while let Some(page) = pager.next_page().await {
            pages.push(page?);
        }
        Ok(pages)
    }
}

impl<T> Paged<T>
where
    T: FromJson + PageItems + Send + 'static,
{
    /// Collect items across pages (`limit` = maximum number of items).
    pub async fn collect_items(self, limit: Option<usize>) -> Result<Vec<T::Item>> {
        let mut items = Vec::new();
        if limit == Some(0) {
            return Ok(items);
        }
        let mut pager = self.pager(None);
        while let Some(page) = pager.next_page().await {
            for item in page?.into_items() {
                items.push(item);
                if limit.is_some_and(|limit| items.len() >= limit) {
                    return Ok(items);
                }
            }
        }
        Ok(items)
    }
}

impl<T: FromJson + Send + 'static> IntoFuture for Paged<T> {
    type Output = Result<T>;
    type IntoFuture = BoxFuture<Result<T>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.send())
    }
}

/// Iterator over pages.
pub struct Pager<T> {
    next: Option<CgiRequest<T>>,
    strategy: Arc<dyn PagerStrategy<T>>,
    limit: Option<usize>,
    yielded: usize,
}

impl<T> std::fmt::Debug for Pager<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pager")
            .field("yielded", &self.yielded)
            .field("limit", &self.limit)
            .finish_non_exhaustive()
    }
}

impl<T: FromJson + Send + 'static> Pager<T> {
    /// Whether another page may be fetched.
    pub fn has_more(&self) -> bool {
        self.next.is_some() && self.limit.is_none_or(|limit| self.yielded < limit)
    }

    /// Number of pages fetched so far.
    pub fn pages_fetched(&self) -> usize {
        self.yielded
    }

    /// Fetch the next page.
    pub async fn next_page(&mut self) -> Option<Result<T>> {
        if !self.has_more() {
            return None;
        }
        let request = self.next.take()?;
        let params = request.spec().param.clone();
        let template = request.clone();
        let response = match request.send().await {
            Ok(response) => response,
            Err(err) => return Some(Err(err)),
        };
        self.yielded += 1;
        match self.strategy.next_params(&params, &response) {
            Ok(Some(next_params)) => {
                let mut next = template;
                next.spec_mut().param = next_params;
                self.next = Some(next);
            }
            Ok(None) => {}
            Err(err) => return Some(Err(err)),
        }
        Some(Ok(response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Debug, Clone, Default)]
    struct Resp {
        has_more: Option<bool>,
        total: Option<i64>,
        count: Option<i64>,
        cursor: Option<Value>,
    }

    fn page() -> PageStrategy<Resp> {
        PageStrategy::new("page")
            .has_more(|r: &Resp| r.has_more)
            .total(|r: &Resp| r.total)
            .count(|r: &Resp| r.count)
    }

    #[test]
    fn page_strategy_rules() {
        let strategy = page().page_size(10);
        let params = json!({"page": 1, "q": "x"});
        let next = strategy
            .next_params(&params, &Resp { has_more: Some(true), ..Resp::default() })
            .unwrap()
            .unwrap();
        assert_eq!(next, json!({"page": 2, "q": "x"}));
        assert!(strategy.next_params(&params, &Resp { has_more: Some(false), total: Some(100), ..Resp::default() }).unwrap().is_none());
        // total based
        assert!(strategy.next_params(&json!({"page": 9}), &Resp { total: Some(100), ..Resp::default() }).unwrap().is_some());
        assert!(strategy.next_params(&json!({"page": 10}), &Resp { total: Some(100), ..Resp::default() }).unwrap().is_none());
        // count based
        assert!(strategy.next_params(&params, &Resp { count: Some(10), ..Resp::default() }).unwrap().is_some());
        assert!(strategy.next_params(&params, &Resp { count: Some(9), ..Resp::default() }).unwrap().is_none());
        assert!(strategy.next_params(&params, &Resp::default()).unwrap().is_none());
        // missing page key uses start page
        let next = strategy.next_params(&json!({}), &Resp { has_more: Some(true), ..Resp::default() }).unwrap();
        assert_eq!(next.unwrap()["page"], 2);
        assert!(strategy.next_params(&json!({"page": "a"}), &Resp { total: Some(5), ..Resp::default() }).is_err());
        let no_size = page();
        assert!(no_size.next_params(&params, &Resp { count: Some(1), ..Resp::default() }).unwrap().is_some());
        assert!(no_size.next_params(&params, &Resp { count: Some(0), ..Resp::default() }).unwrap().is_none());
        let zero_based = PageStrategy::<Resp>::new("p").start_page(0).page_size(5).total(|r: &Resp| r.total);
        assert!(zero_based.next_params(&json!({"p": 0}), &Resp { total: Some(6), ..Resp::default() }).unwrap().is_some());
        assert!(zero_based.next_params(&json!({"p": 1}), &Resp { total: Some(6), ..Resp::default() }).unwrap().is_none());
    }

    #[test]
    fn offset_strategy_rules() {
        let strategy = OffsetStrategy::with_size_key("begin", "num")
            .total(|r: &Resp| r.total)
            .count(|r: &Resp| r.count)
            .has_more(|r: &Resp| r.has_more);
        let params = json!({"begin": 0, "num": 10});
        let next = strategy.next_params(&params, &Resp { total: Some(25), ..Resp::default() }).unwrap();
        assert_eq!(next.unwrap()["begin"], 10);
        let next = strategy.next_params(&params, &Resp { total: Some(25), count: Some(4), ..Resp::default() }).unwrap();
        assert_eq!(next.unwrap()["begin"], 4);
        assert!(strategy.next_params(&json!({"begin": 20, "num": 10}), &Resp { total: Some(25), count: Some(5), ..Resp::default() }).unwrap().is_none());
        assert!(strategy.next_params(&params, &Resp { total: Some(25), count: Some(0), ..Resp::default() }).unwrap().is_none());
        assert!(strategy.next_params(&params, &Resp { count: Some(10), ..Resp::default() }).unwrap().is_some());
        assert!(strategy.next_params(&params, &Resp { count: Some(3), ..Resp::default() }).unwrap().is_none());
        assert!(strategy.next_params(&params, &Resp { has_more: Some(false), total: Some(100), ..Resp::default() }).unwrap().is_none());
        // has_more true but zero step -> error
        assert!(strategy.next_params(&params, &Resp { has_more: Some(true), count: Some(0), ..Resp::default() }).is_err());
        // missing page size
        assert!(strategy.next_params(&json!({"begin": 0}), &Resp { count: Some(3), ..Resp::default() }).is_err());
        let fixed = OffsetStrategy::<Resp>::with_size("offset", 20).start_offset(5).has_more(|r: &Resp| r.has_more);
        let next = fixed.next_params(&json!({}), &Resp { has_more: Some(true), ..Resp::default() }).unwrap();
        assert_eq!(next.unwrap()["offset"], 25);
        assert!(fixed.next_params(&json!({"offset": "x"}), &Resp { has_more: Some(true), ..Resp::default() }).is_err());
    }

    #[test]
    fn cursor_strategy_rules() {
        let strategy = CursorStrategy::new("cursor", |r: &Resp| r.cursor.clone())
            .has_more(|r: &Resp| r.has_more)
            .count(|r: &Resp| r.count)
            .page_size(10);
        let params = json!({"cursor": "a"});
        let next = strategy.next_params(&params, &Resp { cursor: Some(json!("b")), ..Resp::default() }).unwrap();
        assert_eq!(next.unwrap()["cursor"], "b");
        assert!(strategy.next_params(&params, &Resp { cursor: Some(json!("a")), ..Resp::default() }).unwrap().is_none());
        assert!(strategy.next_params(&params, &Resp { cursor: None, ..Resp::default() }).unwrap().is_none());
        assert!(strategy.next_params(&params, &Resp { cursor: Some(Value::Null), ..Resp::default() }).unwrap().is_none());
        assert!(strategy.next_params(&params, &Resp { has_more: Some(false), cursor: Some(json!("b")), ..Resp::default() }).unwrap().is_none());
        assert!(strategy.next_params(&params, &Resp { count: Some(3), cursor: Some(json!("b")), ..Resp::default() }).unwrap().is_none());
        assert!(strategy.next_params(&params, &Resp { count: Some(10), cursor: Some(json!("b")), ..Resp::default() }).unwrap().is_some());
        let repeat = CursorStrategy::new("cursor", |r: &Resp| r.cursor.clone()).allow_repeat(true);
        assert!(repeat.next_params(&params, &Resp { cursor: Some(json!("a")), ..Resp::default() }).unwrap().is_some());
        let zero = CursorStrategy::new("c", |r: &Resp| r.cursor.clone()).count(|r: &Resp| r.count);
        assert!(zero.next_params(&json!({}), &Resp { count: Some(0), cursor: Some(json!(1)), ..Resp::default() }).unwrap().is_none());
    }

    #[test]
    fn fn_strategy_and_with_param() {
        let strategy = FnStrategy(|params: &Value, r: &Resp| r.has_more.filter(|m| *m).map(|_| with_param(params, "x", json!(1))));
        assert_eq!(
            strategy.next_params(&json!({"a": 1}), &Resp { has_more: Some(true), ..Resp::default() }).unwrap(),
            Some(json!({"a": 1, "x": 1}))
        );
        assert!(strategy.next_params(&json!({}), &Resp::default()).unwrap().is_none());
        assert_eq!(with_param(&Value::Null, "k", json!(2)), json!({"k": 2}));
    }
}
