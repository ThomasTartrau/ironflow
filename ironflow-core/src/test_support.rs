use std::sync::{Arc, Mutex};

use crate::provider::LogSink;

/// In-memory [`LogSink`] that collects all lines for assertions.
pub(crate) struct VecSink(pub Mutex<Vec<(String, String)>>);

impl VecSink {
    pub fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(Vec::new())))
    }
}

impl LogSink for VecSink {
    fn log(&self, stream: &str, line: &str) {
        self.0
            .lock()
            .unwrap()
            .push((stream.to_string(), line.to_string()));
    }
}
