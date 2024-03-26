use std::{net::Ipv4Addr, time::Duration};

use trale::{
    futures::{timer::Timer, udp::UdpSocket},
    task::Executor,
};

fn main() {
    let task1 = Executor::spawn(async {
        Timer::sleep(Duration::from_millis(500)).await;
        println!("Hello A!");
        Timer::sleep(Duration::from_secs(1)).await;
        println!("Hello B!");
        Timer::sleep(Duration::from_secs(1)).await;
        println!("Hello C!");
    });

    let task2 = Executor::spawn(async {
        let mut buf = [0u8; 1500];
        let mut udpsock = UdpSocket::bind((Ipv4Addr::LOCALHOST, 9998)).unwrap();
        let (len, src) = udpsock.recv_from(&mut buf).await.unwrap();

        println!("Received {} bytes from {:?}", len, src);
    });

    let task3 = Executor::spawn(async {
        let mut buf = [0xadu8; 20];
        let udpsock = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        Timer::sleep(Duration::from_secs(1)).await;
        let len = udpsock
            .send_to(&mut buf, (Ipv4Addr::LOCALHOST, 9998))
            .await
            .unwrap();

        println!("Sent {} bytes", len);
    });

    task1.join();
    task2.join();
    task3.join();
}
