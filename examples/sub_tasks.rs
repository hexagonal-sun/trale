use std::time::Duration;

use trale::{futures::timer::Timer, task::Executor};

fn main() {
    let task1 = Executor::spawn(async {
        Timer::sleep(Duration::from_secs(2)).unwrap().await;
        println!("Hello A!");

        let task2 = Executor::spawn(async {
            println!("Hello from other task");
            Timer::sleep(Duration::from_secs(1)).unwrap().await;
            println!("Bye bye from other task");

            24
        });

        Timer::sleep(Duration::from_secs(1)).unwrap().await;
        println!("Hello B!");

        assert_eq!(task2.await, 24);

        Timer::sleep(Duration::from_secs(1)).unwrap().await;
        println!("Hello C!");
    });

    task1.join();
}
