pub mod multishot;
pub mod oneshot;

fn reactor_value_to_result(v: i32) -> std::io::Result<i32> {
    if v < 0 {
        Err(std::io::Error::from_raw_os_error(v.abs()))
    } else {
        Ok(v)
    }
}
