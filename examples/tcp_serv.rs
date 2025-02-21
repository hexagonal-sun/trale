use std::time::Duration;

use anyhow::{Context, Result};
use log::debug;
use tokio_stream::StreamExt;
use trale::{
    futures::{read::AsyncRead, tcp::TcpListener, timer::Timer, write::AsyncWrite},
    task::{Executor, TaskJoiner},
};

fn main() -> Result<()> {
    env_logger::init();

    Executor::spawn(async {
        Timer::sleep(Duration::from_millis(500)).unwrap().await;
        println!("Hello A!");
        Timer::sleep(Duration::from_secs(1)).unwrap().await;
        println!("Hello B!");
        Timer::sleep(Duration::from_secs(1)).unwrap().await;
        println!("Hello C!");
        Timer::sleep(Duration::from_secs(1)).unwrap().await;
        println!("Hello D!");
    });

    let echo_task: TaskJoiner<Result<usize>> = Executor::spawn(async {
        let mut buf = [0u8; 1];
        let mut bytes_read: usize = 0;
        let mut listener = TcpListener::bind("127.0.0.1:5000").context("Could not bind")?;

        println!("Waiting for connection on 127.0.0.1:5000");

        let mut conn = listener
            .next()
            .await
            .unwrap()
            .context("Could not accept incoming connection")?;

        // We only want to accept a single connection. Drop the lisetner once
        // we've accepted a connection.
        drop(listener);

        loop {
            debug!("Reading from socket");
            let len = conn.read(&mut buf).await?;
            if len == 0 {
                return Ok(bytes_read);
            }
            bytes_read += 1;
            conn.write(&buf).await?;
        }
    });

    Executor::run();

    let bytes_read = echo_task.join()?;
    eprintln!("Conversation finished.  Read {bytes_read} bytes");

    Ok(())
}
