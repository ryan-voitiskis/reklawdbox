mod audio;
mod beatport;
mod changes;
mod color;
mod corpus;
mod db;
mod discogs;
mod eval_routing;
mod eval_tasks;
mod genre;
mod store;
mod tools;
mod types;
mod xml;

use rmcp::ServiceExt;
use rmcp::transport::stdio;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = tools::ReklawdboxServer::new(db::resolve_db_path());
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
