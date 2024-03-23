use std::time::Duration;

use trale::{futures::timer::Timer, task::Executor};

fn main() {
    let task1 = Executor::spawn(async {
        Timer::sleep(Duration::from_secs(2)).await;
        println!("Hello A!");
        Timer::sleep(Duration::from_secs(1)).await;
        println!("Hello B!");
        Timer::sleep(Duration::from_secs(1)).await;
        println!("Hello C!");
    });

    let task2 = Executor::spawn(async {
        Timer::sleep(Duration::from_secs(2)).await;
        println!("Hello a!");
        Timer::sleep(Duration::from_secs(1)).await;
        println!("Hello b!");
        Timer::sleep(Duration::from_secs(1)).await;
        println!("Hello c!");
    });

    task1.join();
    task2.join();
}
