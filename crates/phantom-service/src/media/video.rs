use std::{
    borrow::Cow,
    fs::{read_dir, remove_file},
    io,
    path::Path,
    process::Stdio,
    time::Duration,
};

use futures::future::try_join;
#[cfg(unix)]
use libc::{SIGKILL, killpg};
use lru_cache::LruCache;
use phantom_core::{Config, Err, Result, debug, debug_warn, defer, err, rand};
use ruma::MxcUri;
use tokio::{
    fs::{OpenOptions, create_dir_all},
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt, copy, sink},
    process::Command,
    time::{Instant, timeout_at},
};

use super::{Service, thumbnail::Dim};

const INPUT: &str = "{input}";
const WIDTH: &str = "{width}";
const HEIGHT: &str = "{height}";

const VIDEO: &str = "video/";

const STAGED: &str = "phantom-video-";

const NAME_LENGTH: usize = 16;

const DIAGNOSTIC_LEN: u64 = 4096;

const FAILURE_COOLDOWN: Duration = Duration::from_secs(5 * 60);

pub(super) const FAILURES: usize = 1024;

const FAIR_SHARE: f64 = 4.0;

pub(super) type Failures = LruCache<String, Instant>;

struct Reaper {
    group: Option<u32>,
}

impl Reaper {
    fn disarm(&mut self) {
        self.group = None;
    }
}

impl Drop for Reaper {
    fn drop(&mut self) {
        if let Some(group) = self.group {
            kill_group(group);
        }
    }
}

#[phantom_core::implement(Service)]
#[tracing::instrument(
    name = "video",
    level = "debug",
    skip_all,
    fields(?dim, ?content_type),
)]
pub(super) async fn video_frame(
    &self,
    mxc: &MxcUri,
    dim: &Dim,
    content_type: Option<&str>,
    content: &[u8],
) -> Option<Vec<u8>> {
    let config = &self.services.config;

    let is_video = content_type.is_some_and(|content_type| content_type.starts_with(VIDEO));

    let workable = !config.media.media_video_thumbnail_command.is_empty()
        && content.len() <= config.media.media_video_thumbnail_max_size
        && !self.failed_recently(mxc);

    if !is_video || !workable {
        return None;
    }

    self.extract_frame(mxc, dim, content)
        .await
        .inspect_err(|e| debug_warn!(%mxc, "Could not extract a video frame: {e}"))
        .ok()
}

#[phantom_core::implement(Service)]
fn failed_recently(&self, mxc: &MxcUri) -> bool {
    let Some(cooldown) = Instant::now().checked_sub(FAILURE_COOLDOWN) else {
        return false;
    };

    self.video_thumbnail_failures
        .lock()
        .ok()
        .and_then(|mut failures| failures.get_mut(mxc.as_str()).copied())
        .is_some_and(|failed| failed > cooldown)
}

#[phantom_core::implement(Service)]
pub(super) fn remember_failure(&self, mxc: &MxcUri) {
    if let Ok(mut failures) = self.video_thumbnail_failures.lock() {
        failures.insert(mxc.as_str().to_owned(), Instant::now());
    }
}

#[phantom_core::implement(Service)]
#[tracing::instrument(level = "debug", skip(self, content))]
async fn extract_frame(&self, mxc: &MxcUri, dim: &Dim, content: &[u8]) -> Result<Vec<u8>> {
    let config = &self.services.config;
    let timeout = Duration::from_secs(config.media.media_video_thumbnail_timeout);

    let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
        err!(Config(
            "media_video_thumbnail_timeout",
            "The timeout is out of range."
        ))
    })?;

    let Some((program, args)) = config.media.media_video_thumbnail_command.split_first() else {
        return Err!(Config(
            "media_video_thumbnail_command",
            "No program is configured."
        ));
    };

    let Ok(Ok(_slot)) = timeout_at(deadline, self.video_thumbnail_slots.acquire()).await else {
        return Err!("Timed out waiting for a video thumbnail slot.");
    };

    let path = staging_dir(config).join(format!("{STAGED}{}", rand::string(NAME_LENGTH)));

    defer! {{ remove_file(&path).ok(); }}

    let Ok(staged) = timeout_at(deadline, stage(&path, content)).await else {
        return Err!("Timed out staging the video.");
    };

    staged?;

    if deadline.saturating_duration_since(Instant::now()) < timeout.div_f64(FAIR_SHARE) {
        return Err!("Too little of the deadline remained to run the video thumbnail program.");
    }

    let width = dim.width.to_string();
    let height = dim.height.to_string();
    let args = args
        .iter()
        .map(|arg| substitute(arg, &path, &width, &height));

    let limit = u64::try_from(config.media.media_video_thumbnail_max_size).unwrap_or(u64::MAX);
    let frame = run(program, args, limit, deadline).await;

    if frame.is_err() {
        self.remember_failure(mxc);
    }

    frame
}

fn staging_dir(config: &Config) -> Cow<'_, Path> {
    config
        .media
        .media_video_thumbnail_path
        .as_deref()
        .map_or_else(
            || config.database.database_path.join("tmp").into(),
            Cow::Borrowed,
        )
}

#[tracing::instrument(name = "sweep", level = "debug", skip_all)]
pub(super) fn sweep_staging_dir(config: &Config) {
    let Ok(dir) = read_dir(staging_dir(config).as_ref()) else {
        return;
    };

    dir.filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(STAGED))
        })
        .for_each(|path| {
            debug!(?path, "Removing a video staged by a previous run.");
            remove_file(&path).ok();
        });
}

async fn stage(path: &Path, content: &[u8]) -> Result {
    if let Some(dir) = path.parent() {
        create_dir_all(dir).await?;
    }

    let mut options = OpenOptions::new();
    options.create_new(true).write(true);

    #[cfg(unix)]
    options.mode(0o600);

    let mut file = options.open(path).await?;

    file.write_all(content).await?;
    file.flush().await?;

    Ok(())
}

pub(super) fn substitute<'a>(
    arg: &'a str,
    input: &Path,
    width: &str,
    height: &str,
) -> Cow<'a, str> {
    if !arg.contains('{') {
        return Cow::Borrowed(arg);
    }

    arg.replace(INPUT, &input.to_string_lossy())
        .replace(WIDTH, width)
        .replace(HEIGHT, height)
        .into()
}

#[tracing::instrument(
    level = "debug",
    skip(args),
    fields(%program, %limit),
)]
pub(super) async fn run<Args>(
    program: &str,
    args: Args,
    limit: u64,
    deadline: Instant,
) -> Result<Vec<u8>>
where
    Args: IntoIterator<Item: AsRef<str>> + Send,
{
    let mut command = Command::new(program);

    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    for arg in args {
        command.arg(arg.as_ref());
    }

    #[cfg(unix)]
    command.process_group(0);

    let mut child = command.spawn()?;
    let mut reaper = Reaper { group: child.id() };

    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");

    let collect = try_join(
        drain(stdout, limit.saturating_add(1)),
        drain(stderr, DIAGNOSTIC_LEN),
    );

    let outcome = timeout_at(deadline, async {
        let collected = collect.await?;
        let status = child.wait().await?;

        Ok::<_, io::Error>((collected, status))
    })
    .await;

    if child.id().is_none() {
        reaper.disarm();
    }

    let Ok(exited) = outcome else {
        return Err!("Video thumbnail program exceeded its deadline.");
    };

    let ((frame, diagnostic), status) = exited?;
    let diagnostic = String::from_utf8_lossy(&diagnostic);

    debug!(?status, len = %frame.len(), "Video thumbnail program exited.");

    if !status.success() {
        return Err!("Video thumbnail program failed with {status}: {diagnostic}");
    }

    if frame.is_empty() {
        return Err!("Video thumbnail program produced no frame: {diagnostic}");
    }

    if u64::try_from(frame.len()).unwrap_or(u64::MAX) > limit {
        return Err!("Video thumbnail program produced a frame past {limit} bytes.");
    }

    Ok(frame)
}

async fn drain<Pipe>(mut pipe: Pipe, limit: u64) -> Result<Vec<u8>, io::Error>
where
    Pipe: AsyncRead + Unpin + Send,
{
    let mut buf = Vec::new();

    (&mut pipe).take(limit).read_to_end(&mut buf).await?;

    copy(&mut pipe, &mut sink()).await?;

    Ok(buf)
}

#[cfg(unix)]
fn kill_group(group: u32) {
    let Ok(group) = i32::try_from(group) else {
        return;
    };

    #[allow(unsafe_code)]
    unsafe {
        killpg(group, SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_group(_group: u32) {}

#[cfg(test)]
mod tests {
    use std::{path::Path, time::Duration};

    use tokio::time::Instant;

    use super::{run, substitute};

    #[test]
    fn substitutes_every_token_in_an_argument() {
        let arg = substitute(
            "{input}:{width}x{height}",
            Path::new("/tmp/video"),
            "96",
            "64",
        );

        assert_eq!(arg, "/tmp/video:96x64");
    }

    #[test]
    fn borrows_an_argument_without_a_token() {
        let arg = substitute("-loglevel", Path::new("/tmp/video"), "96", "64");

        assert!(matches!(arg, std::borrow::Cow::Borrowed("-loglevel")));
    }

    fn deadline() -> Instant {
        Instant::now()
            .checked_add(Duration::from_secs(30))
            .expect("a deadline thirty seconds out is representable")
    }

    #[tokio::test]
    async fn collects_the_frame_from_standard_output() {
        let frame = run("printf", ["frame"], 1024, deadline()).await.unwrap();

        assert_eq!(frame, b"frame");
    }

    #[tokio::test]
    async fn rejects_a_program_that_writes_no_frame() {
        let error = run("true", [""; 0], 1024, deadline()).await.unwrap_err();

        assert!(error.to_string().contains("no frame"));
    }

    #[tokio::test]
    async fn reports_the_diagnostic_of_a_failing_program() {
        let args = ["-c", "echo no such codec >&2; exit 1"];
        let error = run("sh", args, 1024, deadline()).await.unwrap_err();

        assert!(error.to_string().contains("no such codec"));
    }

    #[tokio::test]
    async fn refuses_a_frame_past_the_limit() {
        let args = ["-c", "printf '%0.sX' $(seq 1 64)"];
        let error = run("sh", args, 32, deadline()).await.unwrap_err();

        assert!(error.to_string().contains("past 32 bytes"));
    }

    #[tokio::test]
    async fn accepts_a_frame_filling_the_limit() {
        let args = ["-c", "printf '%0.sX' $(seq 1 32)"];
        let frame = run("sh", args, 32, deadline()).await.unwrap();

        assert_eq!(frame.len(), 32);
    }

    #[tokio::test]
    async fn kills_the_program_that_exceeds_its_deadline() {
        let past = Instant::now();
        let error = run("sleep", ["30"], 1024, past).await.unwrap_err();

        assert!(error.to_string().contains("deadline"));
    }
}
