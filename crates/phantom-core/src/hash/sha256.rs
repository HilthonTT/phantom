use ring::{
    digest,
    digest::{Context, SHA256, SHA256_OUTPUT_LEN},
};

pub type Digest = [u8; SHA256_OUTPUT_LEN];

#[must_use]
#[tracing::instrument(skip(inputs), level = "trace")]
pub fn delimited<'a, T, I>(mut inputs: I) -> Digest
where
    I: Iterator<Item = T> + 'a,
    T: AsRef<[u8]> + 'a,
{
    let mut ctx = Context::new(&SHA256);
    if let Some(input) = inputs.next() {
        ctx.update(input.as_ref());
        for input in inputs {
            ctx.update(b"\xFF");
            ctx.update(input.as_ref());
        }
    }

    ctx.finish()
        .as_ref()
        .try_into()
        .expect("failed to return Digest buffer")
}

#[must_use]
#[tracing::instrument(skip(inputs), level = "trace")]
pub fn concat<'a, T, I>(inputs: I) -> Digest
where
    I: Iterator<Item = T> + 'a,
    T: AsRef<[u8]> + 'a,
{
    inputs
        .fold(Context::new(&SHA256), |mut ctx, input| {
            ctx.update(input.as_ref());
            ctx
        })
        .finish()
        .as_ref()
        .try_into()
        .expect("failed to return Digest buffer")
}

#[inline]
#[must_use]
#[tracing::instrument(skip(input), level = "trace")]
pub fn hash<T>(input: T) -> Digest
where
    T: AsRef<[u8]>,
{
    digest::digest(&SHA256, input.as_ref())
        .as_ref()
        .try_into()
        .expect("failed to return Digest buffer")
}
