//! Search songs: `cargo run --example search -- 周杰伦`

use qqmusic_api::Client;
use qqmusic_api::modules::search::SearchType;

#[tokio::main]
async fn main() -> qqmusic_api::Result<()> {
    let keyword = std::env::args().nth(1).unwrap_or_else(|| "周杰伦".to_string());
    let client = Client::builder().device_path(".qqmusic/device.json").build()?;

    let page = client.search().search_by_type(&keyword, SearchType::Song).await?;
    println!("共约 {} 条结果", page.total_num);
    for item in page.song {
        let song = &item.song;
        let singers: Vec<&str> = song.singer.iter().map(|s| s.name.as_str()).collect();
        println!("{:>14}  {}  -  {}", song.mid, song.name, singers.join(" / "));
    }
    Ok(())
}
