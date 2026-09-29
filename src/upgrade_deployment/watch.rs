//! What a person sees while an upgrade runs, so a wait never looks frozen.
//!
//! On a terminal, a block redrawn in place: each step, how long it took or
//! has taken, the components ready, and what is waited on now. Anywhere else,
//! as in CI or piped to a file, each step as it starts and ends, and a line
//! every so often while it waits, so a log never looks stalled either.
//!
//! Only the ANSI codes written here, and no colour where `NO_COLOR` is set
//! (no-color.org). The report at the end is the same either way.

use std::ffi::OsString;
use std::io::{IsTerminal as _, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

use super::Progress;

/// The steps of an upgrade, in the order they start. The migration and the
/// components are waited for together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Checks,
    Confirmation,
    Apply,
    Migration,
    Components,
    Cleanup,
}

impl Step {
    pub const ALL: [Step; 6] = [
        Step::Checks,
        Step::Confirmation,
        Step::Apply,
        Step::Migration,
        Step::Components,
        Step::Cleanup,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Step::Checks => "checks",
            Step::Confirmation => "plan and confirmation",
            Step::Apply => "apply",
            Step::Migration => "migration",
            Step::Components => "components",
            Step::Cleanup => "cleanup",
        }
    }
}

/// Told where an upgrade has got to. A closure taking a line hears the lines
/// alone, which is all a test of what was said needs.
pub trait Watch {
    /// A line for the record: a finding, the plan, what is first waited for.
    fn say(&mut self, line: &str);
    fn begin(&mut self, _: Step) {}
    fn end(&mut self, _: Step, _ok: bool) {}
    /// Where the wait stands, each time the cluster is asked.
    fn waiting(&mut self, _: &Progress) {}
    /// Done watching: the last of it shown, before the report or the refusal.
    fn finish(&mut self) {}
}

impl<F: FnMut(&str)> Watch for F {
    fn say(&mut self, line: &str) {
        self(line)
    }
}

// ── What one frame shows ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Pending,
    Running(Duration),
    Done(Duration),
    Failed(Duration),
}

/// Everything the block shows, at one moment.
#[derive(Debug, Clone)]
pub struct Board {
    pub title: String,
    pub elapsed: Duration,
    pub steps: Vec<(Step, Mark)>,
    /// Components ready, of all of them, once the wait has counted them.
    pub ready: Option<(usize, usize)>,
    /// What is waited on now, while something is.
    pub now: Option<String>,
}

/// `4s`, `1m 04s`, `1h 02m`.
pub fn clock(took: Duration) -> String {
    let seconds = took.as_secs();
    match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3599 => format!("{}m {:02}s", seconds / 60, seconds % 60),
        _ => format!("{}h {:02}m", seconds / 3600, seconds % 3600 / 60),
    }
}

const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const BAR: usize = 20;

/// The block's lines, for one frame of the spinner. Pure, so a test reads
/// what a terminal would be shown.
pub fn render(board: &Board, frame: usize, colour: bool) -> Vec<String> {
    let paint = |code: &str, text: &str| {
        if colour && !text.is_empty() {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    };
    let mut lines = vec![format!(
        "{}  {}",
        paint("1", &board.title),
        clock(board.elapsed)
    )];
    for (step, mark) in &board.steps {
        let name = format!("{:<21}", step.name());
        let (sign, name, took) = match mark {
            Mark::Pending => (paint("2", "·"), paint("2", step.name()), None),
            Mark::Running(took) => (
                paint("36", &SPINNER[frame % SPINNER.len()].to_string()),
                name,
                Some(took),
            ),
            Mark::Done(took) => (paint("32", "✓"), name, Some(took)),
            Mark::Failed(took) => (paint("31", "✗"), name, Some(took)),
        };
        let mut line = format!(
            "  {sign} {name} {:>7}",
            took.map(|took| clock(*took)).unwrap_or_default()
        );
        if let (Step::Components, Some((ready, of))) = (step, board.ready) {
            let filled = (ready * BAR).checked_div(of).unwrap_or(0).min(BAR);
            line.push_str(&format!(
                "  ready {ready} of {of}  {}{}",
                paint("32", &"█".repeat(filled)),
                paint("2", &"░".repeat(BAR - filled))
            ));
        }
        lines.push(line.trim_end().to_string());
    }
    if let Some(now) = &board.now {
        lines.push(format!("  waiting on {now}"));
    }
    lines
}

/// When each step began and how it ended, against one clock.
struct Steps {
    started: Instant,
    began: [Option<Instant>; 6],
    ended: [Option<(Duration, bool)>; 6],
    ready: Option<(usize, usize)>,
    now: Option<String>,
}

impl Steps {
    fn new() -> Self {
        Steps {
            started: Instant::now(),
            began: [None; 6],
            ended: [None; 6],
            ready: None,
            now: None,
        }
    }

    fn running(&self, step: Step) -> bool {
        self.began[step as usize].is_some() && self.ended[step as usize].is_none()
    }

    fn begin(&mut self, step: Step) {
        self.began[step as usize] = Some(Instant::now());
        self.ended[step as usize] = None;
    }

    /// How long it took, or None for a step not running, which is ended once.
    fn end(&mut self, step: Step, ok: bool) -> Option<Duration> {
        let began = self.began[step as usize].filter(|_| self.running(step))?;
        let took = began.elapsed();
        self.ended[step as usize] = Some((took, ok));
        Some(took)
    }

    /// Whatever is still running stopped without finishing.
    fn stop(&mut self) {
        for step in Step::ALL {
            self.end(step, false);
        }
    }

    fn waiting(&mut self, progress: &Progress) {
        self.ready = Some(progress.ready);
        self.now = progress.now.clone();
    }

    fn board(&self, title: &str) -> Board {
        let at = Instant::now();
        let mark = |step: Step| match (self.began[step as usize], self.ended[step as usize]) {
            (_, Some((took, true))) => Mark::Done(took),
            (_, Some((took, false))) => Mark::Failed(took),
            (Some(began), None) => Mark::Running(at - began),
            (None, None) => Mark::Pending,
        };
        let waiting = self.running(Step::Migration) || self.running(Step::Components);
        Board {
            title: title.to_string(),
            elapsed: at - self.started,
            steps: Step::ALL.iter().map(|&step| (step, mark(step))).collect(),
            ready: self.ready,
            now: self.now.clone().filter(|_| waiting),
        }
    }
}

// ── On a terminal ───────────────────────────────────────────────────────────

/// The block, redrawn in place: on each change, and every half second for its
/// spinner and its clocks.
pub struct Terminal {
    screen: Arc<Mutex<Screen>>,
    ticker: tokio::task::JoinHandle<()>,
}

struct Screen {
    title: String,
    colour: bool,
    steps: Steps,
    frame: usize,
    /// The block's lines now on the screen, to go back over.
    drawn: usize,
}

impl Screen {
    fn erase(&mut self, out: &mut dyn Write) {
        if self.drawn > 0 {
            let _ = write!(out, "\r\x1b[{}A\x1b[J", self.drawn);
            self.drawn = 0;
        }
    }

    fn draw(&mut self, out: &mut dyn Write) {
        self.erase(out);
        // Not while the person reads the plan and is asked.
        if !self.steps.running(Step::Confirmation) {
            let lines = render(&self.steps.board(&self.title), self.frame, self.colour);
            // Wrapping off while it is drawn: a line wider than the terminal
            // is cut rather than taking two rows, so going back over the
            // block goes back over all of it.
            let _ = write!(out, "\x1b[?7l");
            for line in &lines {
                let _ = writeln!(out, "{line}");
            }
            let _ = write!(out, "\x1b[?7h");
            self.drawn = lines.len();
        }
        let _ = out.flush();
    }
}

impl Terminal {
    pub fn new(title: String, colour: bool) -> Self {
        let screen = Arc::new(Mutex::new(Screen {
            title,
            colour,
            steps: Steps::new(),
            frame: 0,
            drawn: 0,
        }));
        let ticker = tokio::spawn({
            let screen = Arc::clone(&screen);
            async move {
                let mut every = tokio::time::interval(Duration::from_millis(500));
                loop {
                    every.tick().await;
                    {
                        let mut screen = screen.lock().unwrap_or_else(PoisonError::into_inner);
                        screen.frame += 1;
                        screen.draw(&mut std::io::stdout().lock());
                    }
                }
            }
        });
        Terminal { screen, ticker }
    }

    fn with(&self, change: impl FnOnce(&mut Screen, &mut dyn Write)) {
        let mut screen = self.screen.lock().unwrap_or_else(PoisonError::into_inner);
        let mut out = std::io::stdout().lock();
        change(&mut screen, &mut out);
        screen.draw(&mut out);
    }
}

impl Watch for Terminal {
    fn say(&mut self, line: &str) {
        self.with(|screen, out| {
            screen.erase(out);
            let _ = writeln!(out, "{line}");
        });
    }

    fn begin(&mut self, step: Step) {
        self.with(|screen, _| screen.steps.begin(step));
    }

    fn end(&mut self, step: Step, ok: bool) {
        self.with(|screen, _| {
            screen.steps.end(step, ok);
        });
    }

    fn waiting(&mut self, progress: &Progress) {
        self.with(|screen, _| screen.steps.waiting(progress));
    }

    /// The last frame stays, with every step's time, above the report. When
    /// nothing was applied, there is nothing worth keeping and it goes.
    fn finish(&mut self) {
        self.ticker.abort();
        let mut screen = self.screen.lock().unwrap_or_else(PoisonError::into_inner);
        let mut out = std::io::stdout().lock();
        screen.steps.stop();
        if screen.steps.began[Step::Apply as usize].is_some() {
            screen.draw(&mut out);
        } else {
            screen.erase(&mut out);
            let _ = out.flush();
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.ticker.abort();
    }
}

// ── Anywhere else ───────────────────────────────────────────────────────────

/// Lines, where nothing can be redrawn: each step as it starts and ends, and
/// while it waits, one every `every` saying what for.
pub struct Log<F: FnMut(&str)> {
    out: F,
    every: Duration,
    steps: Steps,
    /// When a line last said what is waited for, or the wait began.
    said: Option<Instant>,
}

impl<F: FnMut(&str)> Log<F> {
    pub fn new(every: Duration, out: F) -> Self {
        Log {
            out,
            every,
            steps: Steps::new(),
            said: None,
        }
    }

    fn at(&self) -> String {
        format!("[{}]", clock(self.steps.started.elapsed()))
    }
}

impl<F: FnMut(&str)> Watch for Log<F> {
    fn say(&mut self, line: &str) {
        (self.out)(line)
    }

    fn begin(&mut self, step: Step) {
        self.steps.begin(step);
        let line = format!("{} {}: started", self.at(), step.name());
        (self.out)(&line);
    }

    fn end(&mut self, step: Step, ok: bool) {
        let Some(took) = self.steps.end(step, ok) else {
            return;
        };
        let how = if ok { "done in" } else { "failed after" };
        let line = format!("{} {}: {how} {}", self.at(), step.name(), clock(took));
        (self.out)(&line);
    }

    fn waiting(&mut self, progress: &Progress) {
        self.steps.waiting(progress);
        let now = Instant::now();
        match self.said {
            Some(said) if now - said < self.every => {}
            Some(_) => {
                self.said = Some(now);
                let (ready, of) = progress.ready;
                let line = format!(
                    "{} still waiting, {ready} of {of} components ready: {}",
                    self.at(),
                    progress.now.as_deref().unwrap_or("nothing named")
                );
                (self.out)(&line);
            }
            None => self.said = Some(now),
        }
    }

    fn finish(&mut self) {
        self.steps.stop();
    }
}

// ── Which ───────────────────────────────────────────────────────────────────

/// no-color.org: set, and not empty, is no colour.
pub fn colour(no_color: Option<OsString>) -> bool {
    no_color.is_none_or(|said| said.is_empty())
}

/// How often a log says what it still waits for.
pub const EVERY: Duration = Duration::from_secs(20);

/// What watches an upgrade from this command: the block where stdout is a
/// terminal that can draw one, and lines anywhere else.
pub fn to_stdout(title: String) -> Box<dyn Watch> {
    let dumb = std::env::var_os("TERM").is_some_and(|term| term == "dumb");
    if std::io::stdout().is_terminal() && !dumb {
        Box::new(Terminal::new(title, colour(std::env::var_os("NO_COLOR"))))
    } else {
        Box::new(Log::new(EVERY, |line: &str| println!("{line}")))
    }
}
