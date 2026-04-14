use claurst_core::history;

#[tokio::main]
async fn main() {
    let sessions = history::list_sessions().await;
    println!("Found {} sessions:", sessions.len());
    for s in &sessions {
        println!("  {} - {} ({} msgs)", 
            &s.id[..8.min(s.id.len())], 
            s.title.as_deref().unwrap_or("(untitled)"),
            s.messages.len()
        );
    }
}
