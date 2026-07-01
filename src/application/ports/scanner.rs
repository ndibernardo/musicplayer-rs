use std::sync::mpsc::Receiver;

pub trait Scanner {
    fn scan(&self) -> Receiver<Result<u32, String>>;
}
