use std::time::Duration;

use trale::{futures::timer::Timer, task::Executor};

fn main() {
    env_logger::init();

    Executor::spawn(async {
        Timer::sleep(Duration::from_secs(2)).unwrap().await;
        println!("Hello A!");
        Timer::sleep(Duration::from_secs(1)).unwrap().await;
        println!("Hello B!");
        Timer::sleep(Duration::from_secs(1)).unwrap().await;
        println!("Hello C!");
    });

    Executor::spawn(async {
        Timer::sleep(Duration::from_secs(2)).unwrap().await;
        println!("Hello a!");
        Timer::sleep(Duration::from_secs(1)).unwrap().await;
        println!("Hello b!");
        Timer::sleep(Duration::from_secs(1)).unwrap().await;
        println!("Hello c!");
    });

    Executor::run();
}
