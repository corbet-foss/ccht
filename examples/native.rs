//! Stream from an operator-supplied, already authenticated ACP executable.
//!
//! Example: cargo run --example native -- opencode acp

use ccht::native::{AgentCommand, NativeClient, NativeOptions, SessionOptions};
use ccht::{Event, Prompt, acp};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let executable = arguments
        .next()
        .ok_or("supply an installed ACP executable")?;
    let directory = std::env::current_dir()?;
    let command = AgentCommand::new(executable)
        .args(arguments)
        .with_working_directory(&directory)?;
    let client = NativeClient::connect(command, NativeOptions::default()).await?;
    let mut session = client.new_session(SessionOptions::new(directory)).await?;
    let handle = session.handle();
    let mut events = session.take_events()?;
    let reader = tokio::spawn(async move {
        while let Some(message) = events.recv().await {
            match message.event {
                Event::Update {
                    update: acp::SessionUpdate::AgentMessageChunk(chunk),
                } => {
                    if let acp::ContentBlock::Text(text) = chunk.content {
                        print!("{}", text.text);
                    }
                }
                Event::Completed { .. } | Event::Error { .. } => break,
                _ => {}
            }
        }
    });
    let result = handle
        .prompt(Prompt::text("example-turn", "Say hello in one sentence."))
        .await;
    client.close().await?;
    reader.await?;
    result?;
    println!();
    Ok(())
}
