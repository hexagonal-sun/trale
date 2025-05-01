use std::{net::Ipv4Addr, time::Duration};

use trale::{
    futures::{timer::Timer, udp::UdpSocket},
    task::Executor,
};

fn main() {
    Executor::spawn(async {
        Timer::sleep(Duration::from_millis(500)).unwrap().await;
        println!("Hello A!");
        Timer::sleep(Duration::from_secs(1)).unwrap().await;
        println!("Hello B!");
        Timer::sleep(Duration::from_secs(1)).unwrap().await;
        println!("Hello C!");
    });

    Executor::spawn(async {
        let mut buf = [0u8; 1500];
        let mut udpsock = UdpSocket::bind((Ipv4Addr::LOCALHOST, 9998)).unwrap();
        let (len, src) = udpsock.recv_from(&mut buf).await.unwrap();

        println!("Received {} bytes from {:?}", len, src);
    });

    Executor::spawn(async {
        let buf = [0xadu8; 20];
        let udpsock = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        Timer::sleep(Duration::from_secs(1)).unwrap().await;
        let len = udpsock
            .send_to(&buf, (Ipv4Addr::LOCALHOST, 9998))
            .await
            .unwrap();

        println!("Sent {} bytes", len);
    });

    Executor::run()
}
