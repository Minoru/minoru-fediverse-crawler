use tokio::runtime::Runtime;

pub fn main(host: String) -> anyhow::Result<()> {
    let rt = Runtime::new()?;
    rt.block_on(async_main(&host))
}

async fn async_main(host: &str) -> anyhow::Result<()> {
    println!("Checking Fediverse instance at {}", host);
    Ok(())
}
