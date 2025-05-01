use anyhow::Context;
use clap::Parser;
use core::str;
use log::{debug, error};
use std::{
    io::{ErrorKind, Seek},
    net::Ipv4Addr,
    path::PathBuf,
    sync::OnceLock,
};
use tokio_stream::StreamExt;
use trale::{
    futures::{
        fs::File,
        read::AsyncRead,
        tcp::{TcpListener, TcpStream},
        write::AsyncWrite,
    },
    task::Executor,
};

#[derive(Clone)]
enum Response {
    NotImplemented,
    ServerError,
    Ok,
    NotFound,
}

static ARGS: OnceLock<Args> = OnceLock::new();

impl From<&Response> for u32 {
    fn from(value: &Response) -> Self {
        match value {
            Response::NotImplemented => 501,
            Response::ServerError => 500,
            Response::Ok => 200,
            Response::NotFound => 404,
        }
    }
}

impl From<&Response> for &str {
    fn from(value: &Response) -> Self {
        match value {
            Response::NotImplemented => "Not Implemented",
            Response::ServerError => "Internal Server Error",
            Response::Ok => "OK",
            Response::NotFound => "Not Found",
        }
    }
}

/// A trale webserver example.
///
/// This example will server out files that live under `webroot` via the HTTP.
#[derive(Parser, Debug)]
struct Args {
    /// The port number that the web server should listen on.
    #[arg(short, long, default_value_t = 80)]
    port: u16,

    /// The base directory where html files should be searched.
    webroot: PathBuf,
}

async fn send_response_hdr(
    conn: &mut TcpStream,
    code: Response,
    content_length: usize,
) -> anyhow::Result<()> {
    let response = format!(
        "HTTP/1.1 {} {}\r\nServer: tws\r\nContent-Length: {}\r\n\r\n",
        <&Response as Into<u32>>::into(&code),
        <&Response as Into<&str>>::into(&code),
        content_length,
    );

    Ok(conn.write(response.as_bytes()).await.map(|_| ())?)
}

async fn send_file(mut conn: TcpStream, path: PathBuf) -> anyhow::Result<()> {
    match File::open(path).await {
        Ok(mut f) => {
            let len = f.seek(std::io::SeekFrom::End(0))? as usize;

            f.seek(std::io::SeekFrom::Start(0))?;

            send_response_hdr(&mut conn, Response::Ok, len).await?;

            loop {
                let mut buf = [0; 4096];
                let len = f.read(&mut buf).await?;

                if len == 0 {
                    break;
                }

                conn.write(&buf).await?;
            }

            Ok(())
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            send_response_hdr(&mut conn, Response::NotFound, 0).await
        }
        _ => send_response_hdr(&mut conn, Response::ServerError, 0).await,
    }
}

async fn handle_connection(mut conn: TcpStream) -> anyhow::Result<()> {
    debug!("New Connection");
    let mut buf = [0; 1024];
    let mut request = String::new();

    while !request.contains("\r\n\r\n") {
        let len = conn.read(&mut buf).await?;
        request.push_str(str::from_utf8(&buf[..len]).unwrap());
    }

    debug!("Got request: {}", request);

    let req_hdr = request.split("\n").next().unwrap().trim();

    let parts: Vec<_> = req_hdr.split(" ").collect();

    let (method, path) = (parts[0], parts[1]);

    if method.to_lowercase() != "get" {
        return send_response_hdr(&mut conn, Response::NotImplemented, 0).await;
    }

    let path = PathBuf::from(path);

    let file = if path == PathBuf::from("/") {
        ARGS.get().unwrap().webroot.join("index.html")
    } else if let Ok(path) = path.strip_prefix("/") {
        ARGS.get().unwrap().webroot.join(path)
    } else {
        return send_response_hdr(&mut conn, Response::NotFound, 0).await;
    };

    send_file(conn, file).await
}

fn main() -> anyhow::Result<()> {
    env_logger::init();

    let args = Args::parse();

    ARGS.set(args).expect("Should have never been set");

    let mut listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, ARGS.get().unwrap().port))
        .context("Could not setup socket listener")?;

    Executor::block_on(async move {
        while let Some(conn) = listener.next().await {
            match conn {
                Ok(conn) => {
                    Executor::spawn(async {
                        if let Err(e) = handle_connection(conn).await {
                            error!("Error handling connection: {e:#}");
                        }
                    });
                }
                Err(e) => error!("Could not accept incoming connection: {e:?}"),
            }
        }
        eprintln!("Bye!");
    });

    Ok(())
}
