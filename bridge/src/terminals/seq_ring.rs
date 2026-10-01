use std::collections::VecDeque;
use tokio::sync::watch;

pub struct SeqRing<T: Clone> {
    chunks: VecDeque<(u64, T, usize)>,
    capacity_bytes: Option<usize>,
    capacity_count: Option<usize>,
    current_bytes: usize,
    next_seq: u64,
    watch_tx: watch::Sender<u64>,
    watch_rx: watch::Receiver<u64>,
}

impl<T: Clone> SeqRing<T> {
    pub fn new(capacity_bytes: Option<usize>, capacity_count: Option<usize>) -> Self {
        let (watch_tx, watch_rx) = watch::channel(0);
        Self {
            chunks: VecDeque::new(),
            capacity_bytes,
            capacity_count,
            current_bytes: 0,
            next_seq: 1,
            watch_tx,
            watch_rx,
        }
    }

    pub fn push<F>(&mut self, f: F, size_bytes: usize) -> u64
    where
        F: FnOnce(u64) -> T,
    {
        let seq = self.next_seq;
        self.next_seq += 1;

        let item = f(seq);
        self.chunks.push_back((seq, item, size_bytes));
        self.current_bytes += size_bytes;

        while self.chunks.len() > 1 && self.needs_eviction() {
            if let Some((_, _, s)) = self.chunks.pop_front() {
                self.current_bytes = self.current_bytes.saturating_sub(s);
            }
        }

        let _ = self.watch_tx.send(seq);
        seq
    }

    fn needs_eviction(&self) -> bool {
        if let Some(max_c) = self.capacity_count
            && self.chunks.len() > max_c
        {
            return true;
        }
        if let Some(max_b) = self.capacity_bytes
            && self.current_bytes > max_b
        {
            return true;
        }
        false
    }

    pub fn first_seq(&self) -> u64 {
        self.chunks
            .front()
            .map(|(s, _, _)| *s)
            .unwrap_or(self.next_seq)
    }

    pub fn read_after(&self, cursor: u64, max: usize) -> (Vec<(u64, T)>, Option<u64>) {
        let first = self.first_seq();
        let truncated = if cursor < first.saturating_sub(1) {
            Some(first - 1 - cursor)
        } else {
            None
        };

        let vec: Vec<(u64, T)> = self
            .chunks
            .iter()
            .filter(|(s, _, _)| *s > cursor)
            .take(max)
            .map(|(s, item, _)| (*s, item.clone()))
            .collect();

        (vec, truncated)
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.watch_rx.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_seq_ring_push_and_read() {
        let mut ring = SeqRing::new(None, Some(3));
        assert_eq!(ring.first_seq(), 1);
        ring.push(|_| "A", 1);
        ring.push(|_| "B", 1);

        let (res, trunc) = ring.read_after(0, 10);
        assert_eq!(trunc, None);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].0, 1);
        assert_eq!(res[0].1, "A");
    }

    #[test]
    fn test_seq_ring_truncation() {
        let mut ring = SeqRing::new(None, Some(2));
        ring.push(|_| "A", 1);
        ring.push(|_| "B", 1);
        ring.push(|_| "C", 1); // A evicted

        assert_eq!(ring.first_seq(), 2);

        let (res, trunc) = ring.read_after(0, 10);
        assert_eq!(trunc, Some(1));
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].0, 2);
        assert_eq!(res[0].1, "B");
    }

    #[test]
    fn test_seq_ring_bytes_eviction() {
        let mut ring = SeqRing::new(Some(5), None);
        ring.push(|_| "A", 3);
        ring.push(|_| "B", 2); // total 5, OK
        ring.push(|_| "C", 1); // total 6, evicts A, remaining 3, OK

        assert_eq!(ring.first_seq(), 2);
    }

    #[test]
    fn test_seq_ring_watch() {
        let mut ring = SeqRing::new(None, Some(3));
        let mut rx = ring.subscribe();
        assert_eq!(*rx.borrow(), 0);
        ring.push(|_| "A", 1);
        assert!(rx.has_changed().unwrap());
        assert_eq!(*rx.borrow_and_update(), 1);
    }
}
