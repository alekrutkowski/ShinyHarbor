//! Socket inactivity deadlines shared by plaintext, PEM and ACME listeners.
use axum_server::accept::Accept;
use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    time::{Instant, Sleep},
};

#[derive(Clone)]
pub struct IdleAcceptor<A> {
    inner: A,
    timeout: Duration,
}
impl<A> IdleAcceptor<A> {
    pub fn new(inner: A, timeout: Duration) -> Self {
        Self { inner, timeout }
    }
}
impl<A, I, S> Accept<I, S> for IdleAcceptor<A>
where
    A: Accept<I, S>,
    A::Future: Send + 'static,
    A::Stream: Send,
    A::Service: Send,
{
    type Stream = IdleIo<A::Stream>;
    type Service = A::Service;
    type Future = Pin<Box<dyn Future<Output = io::Result<(Self::Stream, Self::Service)>> + Send>>;
    fn accept(&self, stream: I, service: S) -> Self::Future {
        let future = self.inner.accept(stream, service);
        let timeout = self.timeout;
        Box::pin(async move {
            let (stream, service) = future.await?;
            Ok((IdleIo::new(stream, timeout), service))
        })
    }
}
pub struct IdleIo<T> {
    inner: T,
    timeout: Duration,
    read_timer: Option<Pin<Box<Sleep>>>,
    write_timer: Option<Pin<Box<Sleep>>>,
}
impl<T> IdleIo<T> {
    fn new(inner: T, timeout: Duration) -> Self {
        Self {
            inner,
            timeout,
            read_timer: (!timeout.is_zero()).then(|| Box::pin(tokio::time::sleep(timeout))),
            write_timer: (!timeout.is_zero()).then(|| Box::pin(tokio::time::sleep(timeout))),
        }
    }
    fn touch(&mut self) {
        if let Some(timer) = &mut self.read_timer {
            timer.as_mut().reset(Instant::now() + self.timeout);
        }
        if let Some(timer) = &mut self.write_timer {
            timer.as_mut().reset(Instant::now() + self.timeout);
        }
    }
    fn deadline(timer: &mut Option<Pin<Box<Sleep>>>, cx: &mut Context<'_>) -> io::Result<()> {
        if timer
            .as_mut()
            .is_some_and(|t| t.as_mut().poll(cx).is_ready())
        {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "HTTP connection inactive",
            ))
        } else {
            Ok(())
        }
    }
}
impl<T: AsyncRead + Unpin> AsyncRead for IdleIo<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Self::deadline(&mut self.read_timer, cx)?;
        let before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        if buf.filled().len() > before {
            self.touch();
        }
        result
    }
}
impl<T: AsyncWrite + Unpin> AsyncWrite for IdleIo<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Self::deadline(&mut self.write_timer, cx)?;
        let result = Pin::new(&mut self.inner).poll_write(cx, buf);
        if matches!(result, Poll::Ready(Ok(n)) if n > 0) {
            self.touch();
        }
        result
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Self::deadline(&mut self.write_timer, cx)?;
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Self::deadline(&mut self.write_timer, cx)?;
        let result = Pin::new(&mut self.inner).poll_write_vectored(cx, bufs);
        if matches!(result, Poll::Ready(Ok(n)) if n > 0) {
            self.touch();
        }
        result
    }
}
