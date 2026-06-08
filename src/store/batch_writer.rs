// file: store/batch_writer.rs
// Purpose:
//   A small, generic write-buffering helper used to coalesce many small writes (SQLite rows,
//   zip entries, ...) into fewer, larger flushes — reducing disk activity and lock churn
//   (roadmap point 9). Flushing is triggered by either a size threshold or an age threshold,
//   checked on each `push`, and is always forced when the writer is dropped (e.g. at
//   shutdown) so no buffered data is lost. The age threshold bounds latency so a trickle of
//   items can never sit buffered indefinitely.
//
//   The flush behaviour is injected as a closure returning `Result<usize, String>` (rows
//   written), so the same buffering logic works for any backing store. Errors during the
//   drop-time flush are logged rather than panicking.

use std::time::{Duration, Instant};
use log::{debug, error};

pub struct BatchWriter<T> {
    buffer: Vec<T>,
    max_batch: usize,
    max_age: Duration,
    last_flush: Instant,
    flush_fn: Box<dyn FnMut(&[T]) -> Result<usize, String> + Send>,
    label: String,
}

impl<T> BatchWriter<T> {
    /// Create a writer that flushes when the buffer reaches `max_batch` items or when
    /// `max_age` has elapsed since the last flush (whichever comes first), or on drop.
    pub fn new<F>(label: &str, max_batch: usize, max_age: Duration, flush_fn: F) -> Self
    where
        F: FnMut(&[T]) -> Result<usize, String> + Send + 'static,
    {
        BatchWriter {
            buffer: Vec::with_capacity(max_batch.max(1)),
            max_batch: max_batch.max(1),
            max_age,
            last_flush: Instant::now(),
            flush_fn: Box::new(flush_fn),
            label: label.to_string(),
        }
    }

    /// Add an item, flushing first if the size or age threshold has been reached.
    pub fn push(&mut self, item: T) -> Result<(), String> {
        if self.buffer.len() >= self.max_batch
            || (!self.buffer.is_empty() && self.last_flush.elapsed() >= self.max_age)
        {
            self.flush()?;
        }
        self.buffer.push(item);
        Ok(())
    }

    /// Flush all buffered items now. No-op when the buffer is empty.
    pub fn flush(&mut self) -> Result<usize, String> {
        if self.buffer.is_empty() {
            self.last_flush = Instant::now();
            return Ok(0);
        }
        let written = (self.flush_fn)(&self.buffer)?;
        debug!("BatchWriter[{}]: flushed {} buffered item(s), wrote {} row(s)",
            self.label, self.buffer.len(), written);
        self.buffer.clear();
        self.last_flush = Instant::now();
        Ok(written)
    }

    /// Number of items currently buffered (not yet flushed).
    pub fn pending(&self) -> usize {
        self.buffer.len()
    }
}

impl<T> Drop for BatchWriter<T> {
    fn drop(&mut self) {
        if !self.buffer.is_empty() {
            if let Err(e) = self.flush() {
                error!("BatchWriter[{}]: final flush on drop failed: {}", self.label, e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Collect flushed batches into a shared vec for assertions.
    fn collector() -> (Arc<Mutex<Vec<usize>>>, impl FnMut(&[i32]) -> Result<usize, String> + Send) {
        let sink: Arc<Mutex<Vec<usize>>> = Arc::new(Mutex::new(Vec::new()));
        let sink_clone = Arc::clone(&sink);
        let f = move |items: &[i32]| {
            sink_clone.lock().unwrap().push(items.len());
            Ok(items.len())
        };
        (sink, f)
    }

    #[test]
    fn test_flush_on_size_threshold() {
        let (sink, f) = collector();
        let mut bw = BatchWriter::new("test", 3, Duration::from_secs(3600), f);
        // push 3 -> the 4th push triggers a flush of the first 3
        for i in 0..4 {
            bw.push(i).unwrap();
        }
        assert_eq!(*sink.lock().unwrap(), vec![3], "should have flushed one batch of 3");
        assert_eq!(bw.pending(), 1);
    }

    #[test]
    fn test_flush_on_drop() {
        let (sink, f) = collector();
        {
            let mut bw = BatchWriter::new("test", 100, Duration::from_secs(3600), f);
            bw.push(1).unwrap();
            bw.push(2).unwrap();
            assert_eq!(bw.pending(), 2);
        } // dropped here -> final flush
        assert_eq!(*sink.lock().unwrap(), vec![2], "drop must flush remaining items");
    }

    #[test]
    fn test_explicit_flush_empty_is_noop() {
        let (sink, f) = collector();
        let mut bw = BatchWriter::new("test", 10, Duration::from_secs(3600), f);
        assert_eq!(bw.flush().unwrap(), 0);
        assert!(sink.lock().unwrap().is_empty());
    }

    #[test]
    fn test_flush_on_age_threshold() {
        let (sink, f) = collector();
        let mut bw = BatchWriter::new("test", 1000, Duration::from_millis(1), f);
        bw.push(1).unwrap();
        std::thread::sleep(Duration::from_millis(5));
        // next push sees the age threshold exceeded and flushes the buffered item first
        bw.push(2).unwrap();
        assert_eq!(*sink.lock().unwrap(), vec![1], "age threshold should have flushed the first item");
    }
}
