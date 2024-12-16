use std::time::Duration;

use anyhow::{Context, Result};
use trale::{
    futures::{read::AsyncRead, tcp::TcpStream, timer::Timer, write::AsyncWrite},
    task::{Executor, TaskJoiner},
};

fn main() -> Result<()> {
    let task1 = Executor::spawn(async {
        Timer::sleep(Duration::from_millis(500)).unwrap().await;
        println!("Hello A!");
        Timer::sleep(Duration::from_secs(1)).unwrap().await;
        println!("Hello B!");
        Timer::sleep(Duration::from_secs(1)).unwrap().await;
        println!("Hello C!");
    });

    let echo_task: TaskJoiner<Result<usize>> = Executor::spawn(async {
        let mut buf = [0u8; 1];
        let mut bytes_read: usize = 0;
        let mut sock = TcpStream::connect("127.0.0.1:5000")
            .await
            .context("Could not connect")?;
        loop {
            let len = sock.read(&mut buf).await?;
            if len == 0 {
                return Ok(bytes_read);
            }
            bytes_read += 1;
            sock.write(&buf).await?;
        }
    });

    let bytes_read = echo_task.join()?;
    eprintln!("Conversation finished.  Read {bytes_read} bytes");
    task1.join();

    Ok(())
}
