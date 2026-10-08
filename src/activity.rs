//! Bounded idle detection for transparent, bidirectional upgraded connections.
use std::{
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

struct Activity<T> {
    io: T,
    started: Instant,
    last: Arc<AtomicU64>,
}
impl<T> Activity<T> {
    fn touch(&self) {
        self.last
            .store(self.started.elapsed().as_millis() as u64, Ordering::Relaxed);
    }
}
impl<T: AsyncRead + Unpin> AsyncRead for Activity<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.io).poll_read(cx, buf);
        if buf.filled().len() > before {
            self.touch();
        }
        result
    }
}
impl<T: AsyncWrite + Unpin> AsyncWrite for Activity<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let result = Pin::new(&mut self.io).poll_write(cx, buf);
        if matches!(result, Poll::Ready(Ok(n)) if n > 0) {
            self.touch();
        }
        result
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }
}
pub async fn copy<A, B>(
    a: &mut A,
    b: &mut B,
    timeout: Option<Duration>,
) -> std::io::Result<(u64, u64)>
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    let Some(timeout) = timeout else {
        return tokio::io::copy_bidirectional(a, b).await;
    };
    let started = Instant::now();
    let last = Arc::new(AtomicU64::new(0));
    let mut a = Activity {
        io: a,
        started,
        last: last.clone(),
    };
    let mut b = Activity {
        io: b,
        started,
        last: last.clone(),
    };
    let expired = async {
        loop {
            let elapsed = started
                .elapsed()
                .saturating_sub(Duration::from_millis(last.load(Ordering::Relaxed)));
            if elapsed >= timeout {
                break;
            }
            tokio::time::sleep(timeout - elapsed).await;
        }
    };
    tokio::select! {
        result = tokio::io::copy_bidirectional(&mut a, &mut b) => result,
        _ = expired => Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "Idle application session")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[tokio::test]
    async fn traffic_in_either_direction_resets_the_idle_deadline() {
        let (mut client, mut a) = tokio::io::duplex(64);
        let (mut b, mut server) = tokio::io::duplex(64);
        let copy = tokio::spawn(async move {
            super::copy(&mut a, &mut b, Some(Duration::from_millis(100))).await
        });
        for i in 0..6 {
            tokio::time::sleep(Duration::from_millis(35)).await;
            let (sender, receiver) = if i % 2 == 0 {
                (&mut client, &mut server)
            } else {
                (&mut server, &mut client)
            };
            sender.write_all(b"ok").await.unwrap();
            let mut data = [0; 2];
            receiver.read_exact(&mut data).await.unwrap();
            assert_eq!(&data, b"ok");
        }
        let error = tokio::time::timeout(Duration::from_secs(2), copy)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    }
}
