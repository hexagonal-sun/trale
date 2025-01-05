use std::{
    mem::MaybeUninit,
    os::fd::{AsFd, AsRawFd},
};

const MAX_NUM_EVENTS: usize = 512;

pub struct Events {
    inner: [MaybeUninit<libc::epoll_event>; MAX_NUM_EVENTS],
}

impl Events {
    pub const fn new() -> Self {
        Self {
            inner: [MaybeUninit::uninit(); MAX_NUM_EVENTS],
        }
    }

    pub fn wait(
        &mut self,
        fd: impl AsFd,
    ) -> std::io::Result<impl Iterator<Item = libc::epoll_event> + '_> {
        let n = unsafe {
            libc::epoll_wait(
                fd.as_fd().as_raw_fd(),
                self.inner.as_mut_ptr() as *mut _,
                MAX_NUM_EVENTS as i32,
                -1,
            )
        };

        if n == -1 {
            return Err(std::io::Error::last_os_error());
        }

        Ok(self
            .inner
            .iter()
            .take(n as usize)
            .map(|x| unsafe { x.assume_init() }))
    }
}
