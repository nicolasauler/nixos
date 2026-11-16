use std::env;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

const SAMPLE_INTERVAL: Duration = Duration::from_secs(2);
/// Temperatures change slowly, and every NVMe read is a command to the drive.
const TEMPERATURE_INTERVAL: Duration = Duration::from_secs(10);
/// A reading this old means its reader is stuck (on an unresponsive NVMe
/// drive, say): the tooltip then shows none rather than a stale value.
const TEMPERATURE_STALE: Duration = Duration::from_secs(30);

const USAGE: &str = "usage: waybar-cores [--hwmon LABEL DIR INPUT]...";

#[derive(Debug, PartialEq, Eq)]
struct Counters([u64; 8]);

impl Counters {
    fn parse<'a>(mut fields: impl Iterator<Item = &'a str>) -> Option<Self> {
        let mut ticks = [0; 8];
        for tick in &mut ticks {
            *tick = fields.next()?.parse().ok()?;
        }
        // user, nice, system, idle, iowait, irq, softirq, steal. The later
        // guest and guest_nice fields are already included in user and nice.
        Some(Self(ticks))
    }

    fn percentage_since(&self, previous: &Self) -> Option<u8> {
        let mut total = 0u64;
        let mut busy = 0u64;
        for (index, (&current, &previous)) in self.0.iter().zip(&previous.0).enumerate() {
            // Check each counter, not just their sum: an individual reset
            // (including a decreasing iowait counter) invalidates this delta.
            let delta = current.checked_sub(previous)?;
            total = total.checked_add(delta)?;
            if index != 3 && index != 4 {
                busy = busy.checked_add(delta)?;
            }
        }
        if total == 0 {
            return None;
        }
        // Round to the nearest integer without overflowing large deltas.
        Some(((u128::from(busy) * 100 + u128::from(total / 2)) / u128::from(total)) as u8)
    }
}

struct Cpu {
    id: u32,
    counters: Option<Counters>,
}

#[derive(Debug, PartialEq, Eq)]
struct Usage {
    id: u32,
    percent: Option<u8>,
}

fn parse_snapshot(input: &str, cpus: &mut Vec<Cpu>) -> bool {
    cpus.clear();
    for line in input.lines() {
        let mut fields = line.split_ascii_whitespace();
        let Some(id) = fields
            .next()
            .and_then(|label| label.strip_prefix("cpu"))
            .filter(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
            .and_then(|id| id.parse().ok())
        else {
            continue;
        };
        cpus.push(Cpu {
            id,
            counters: Counters::parse(fields),
        });
    }
    cpus.sort_unstable_by_key(|cpu| cpu.id);
    !cpus.is_empty() && !cpus.windows(2).any(|pair| pair[0].id == pair[1].id)
}

#[derive(Default)]
struct Sampler {
    previous: Vec<Cpu>,
    current: Vec<Cpu>,
    usages: Vec<Usage>,
}

impl Sampler {
    fn observe(&mut self, input: &str) -> bool {
        if !parse_snapshot(input, &mut self.current) {
            self.unavailable();
            return false;
        }
        self.usages.clear();
        let mut previous_index = 0;
        for cpu in &self.current {
            while previous_index < self.previous.len() && self.previous[previous_index].id < cpu.id
            {
                previous_index += 1;
            }
            let previous = self
                .previous
                .get(previous_index)
                .filter(|old| old.id == cpu.id);
            let percent = cpu.counters.as_ref().and_then(|counters| {
                previous
                    .and_then(|old| old.counters.as_ref())
                    .and_then(|old| counters.percentage_since(old))
            });
            self.usages.push(Usage {
                id: cpu.id,
                percent,
            });
        }
        std::mem::swap(&mut self.previous, &mut self.current);
        true
    }

    fn unavailable(&mut self) {
        // Keep cell identities visible, but never bridge a failed read with
        // an apparently fresh reading or mistake absent data for idle CPUs.
        self.previous.clear();
        for usage in &mut self.usages {
            usage.percent = None;
        }
    }
}

/// One bar per logical CPU, from idle to fully busy.
const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// The nearest of the eight bar heights; percentages never exceed 100.
fn level(percent: u8) -> char {
    LEVELS[(usize::from(percent) * (LEVELS.len() - 1) + 50) / 100]
}

/// A tooltip temperature: `input` (millidegrees Celsius) in the one `hwmon*`
/// entry of `dir`. The `hwmonN` index can change between boots; the device
/// path does not.
#[derive(Debug, PartialEq, Eq)]
struct Hwmon {
    dir: PathBuf,
    input: OsString,
}

impl Hwmon {
    fn read(&self) -> Option<i32> {
        let mut children = fs::read_dir(&self.dir)
            .ok()?
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().as_encoded_bytes().starts_with(b"hwmon"));
        let child = children.next()?;
        // With two candidates, the reading would be a guess.
        if children.next().is_some() {
            return None;
        }
        parse_millidegrees(&fs::read_to_string(child.path().join(&self.input)).ok()?)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Sensor {
    label: String,
    source: Hwmon,
}

/// Labels go into the JSON record verbatim, so they are limited to characters
/// that need no escaping there or in Waybar's markup.
fn valid_label(label: &str) -> bool {
    !label.is_empty()
        && label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b" ()+-./_".contains(&byte))
}

fn parse_args(mut args: impl Iterator<Item = OsString>) -> Result<Vec<Sensor>, String> {
    let mut sensors = Vec::new();
    while let Some(flag) = args.next() {
        let flag = flag.to_string_lossy().into_owned();
        if flag != "--hwmon" {
            return Err(format!("unknown argument {flag}"));
        }
        let mut next = |what: &str| args.next().ok_or_else(|| format!("{flag} needs {what}"));
        let label = next("a label")?;
        let source = Hwmon {
            dir: next("a directory")?.into(),
            input: next("an input file name")?,
        };
        let label = label
            .into_string()
            .ok()
            .filter(|label| valid_label(label))
            .ok_or_else(|| {
                format!("{flag}: a label may only use ASCII letters, digits, spaces and ()+-./_")
            })?;
        sensors.push(Sensor { label, source });
    }
    Ok(sensors)
}

/// Whole degrees Celsius, rounding half up, from a hwmon millidegree value.
fn parse_millidegrees(text: &str) -> Option<i32> {
    let millidegrees: i64 = text.trim().parse().ok()?;
    i32::try_from(millidegrees.checked_add(500)?.div_euclid(1000)).ok()
}

/// The average reported clock, as Waybar's cpu module computes `avg_frequency`
/// for the bar: the mean of every `cpu MHz` line's whole megahertz, rounded up
/// to 10 MHz, kept in single precision as Waybar keeps it.
fn average_ghz(cpuinfo: &str) -> Option<f32> {
    let (mut sum, mut count) = (0_u64, 0_u64);
    for line in cpuinfo.lines().filter(|line| line.starts_with("cpu MHz")) {
        let (_, value) = line.split_once(':')?;
        let whole = value.trim().split('.').next()?;
        sum = sum.checked_add(whole.parse().ok()?)?;
        count += 1;
    }
    if count == 0 {
        return None;
    }
    // Both steps land in a `float` in Waybar, which decides the tenths shown: as
    // an f32, 3.35 lies just below 3.35 (shown 3.3) and 3.15 just above (3.2).
    let mean = (sum as f64 / count as f64) as f32;
    Some(((f64::from(mean) / 10.0).ceil() / 100.0) as f32)
}

struct Readings {
    taken: Instant,
    celsius: Vec<Option<i32>>,
}

/// The readings, unless they are too old to show as current.
fn fresh(readings: Option<&Readings>, now: Instant) -> Option<&[Option<i32>]> {
    readings
        .filter(|readings| now.saturating_duration_since(readings.taken) <= TEMPERATURE_STALE)
        .map(|readings| readings.celsius.as_slice())
}

/// Reads every source once into `celsius` and returns when the batch started: a
/// slow later read (an NVMe sensor read can take the drive's 60 s admin
/// timeout) must never make an earlier reading look newer than it is.
fn read_batch<S>(
    sources: &[S],
    read: impl Fn(&S) -> Option<i32>,
    now: impl Fn() -> Instant,
    celsius: &mut Vec<Option<i32>>,
) -> Instant {
    let started = now();
    celsius.clear();
    celsius.extend(sources.iter().map(read));
    started
}

/// Reads every source each `TEMPERATURE_INTERVAL` on its own thread, so a stuck
/// read never delays the CPU strip.
fn spawn_reader(sources: Vec<Hwmon>) -> Arc<Mutex<Option<Readings>>> {
    let latest = Arc::new(Mutex::new(None));
    let shared = Arc::clone(&latest);
    thread::spawn(move || {
        let mut celsius = Vec::with_capacity(sources.len());
        loop {
            let taken = read_batch(&sources, Hwmon::read, Instant::now, &mut celsius);
            let mut slot = shared.lock().unwrap_or_else(PoisonError::into_inner);
            let readings = slot.get_or_insert_with(|| Readings {
                taken,
                celsius: Vec::with_capacity(sources.len()),
            });
            readings.taken = taken;
            // The previous buffer comes back for the next round.
            std::mem::swap(&mut readings.celsius, &mut celsius);
            drop(slot);
            thread::sleep(TEMPERATURE_INTERVAL);
        }
    });
    latest
}

fn render_frame(
    frame: &mut String,
    usages: &[Usage],
    available: bool,
    ghz: Option<f32>,
    labels: &[String],
    celsius: Option<&[Option<i32>]>,
) {
    frame.clear();
    // JSON contains only these fixed strings, bar glyphs, parsed numeric CPU
    // IDs, computed percentages and clock, validated labels and parsed
    // temperatures. Tooltip line breaks are escaped explicitly.
    frame.push_str("{\"text\":\"");
    if usages.is_empty() {
        frame.push_str("cores unavailable");
    }
    for usage in usages {
        frame.push(usage.percent.map_or('?', level));
    }
    frame.push_str("\",\"tooltip\":\"");
    for (index, label) in labels.iter().enumerate() {
        if index > 0 {
            frame.push_str(", ");
        }
        frame.push_str(label);
        match celsius.and_then(|celsius| celsius.get(index).copied().flatten()) {
            Some(value) => {
                write!(frame, " {value}°C").expect("formatting into a String cannot fail")
            }
            None => frame.push_str(" n/a"),
        }
    }
    if !labels.is_empty() {
        frame.push_str("\\n");
    }
    match ghz {
        Some(ghz) => write!(frame, "Average clock {ghz:.1} GHz\\n")
            .expect("formatting into a String cannot fail"),
        None => frame.push_str("Average clock ?\\n"),
    }
    if available {
        frame.push_str("Logical CPU usage (cell order)");
    } else {
        frame.push_str("CPU data unavailable; cells retain last known IDs");
    }
    let mut pending = false;
    for usage in usages {
        write!(frame, "\\ncpu{}: ", usage.id).expect("formatting into a String cannot fail");
        if let Some(percent) = usage.percent {
            write!(frame, "{percent}%").expect("formatting into a String cannot fail");
        } else {
            frame.push('?');
            pending = true;
        }
    }
    if pending {
        frame.push_str("\\n? = no valid counter delta");
    }
    frame.push_str("\",\"class\":\"");
    frame.push_str(if !available {
        "unavailable"
    } else if pending {
        "pending"
    } else {
        "normal"
    });
    frame.push_str("\"}\n");
}

fn main() -> io::Result<()> {
    let sensors = parse_args(env::args_os().skip(1)).unwrap_or_else(|message| {
        eprintln!("waybar-cores: {message}\n{USAGE}");
        process::exit(2);
    });
    let (labels, sources): (Vec<_>, Vec<_>) = sensors
        .into_iter()
        .map(|sensor| (sensor.label, sensor.source))
        .unzip();
    let temperatures = (!sources.is_empty()).then(|| spawn_reader(sources));
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let mut input = String::new();
    let mut cpuinfo = String::new();
    let mut frame = String::new();
    let mut sampler = Sampler::default();
    loop {
        input.clear();
        let available =
            match File::open("/proc/stat").and_then(|mut file| file.read_to_string(&mut input)) {
                Ok(_) => sampler.observe(&input),
                Err(_) => {
                    sampler.unavailable();
                    false
                }
            };
        cpuinfo.clear();
        let ghz = File::open("/proc/cpuinfo")
            .and_then(|mut file| file.read_to_string(&mut cpuinfo))
            .ok()
            .and_then(|_| average_ghz(&cpuinfo));
        {
            let latest = temperatures
                .as_ref()
                .map(|latest| latest.lock().unwrap_or_else(PoisonError::into_inner));
            let celsius = fresh(latest.as_deref().and_then(Option::as_ref), Instant::now());
            render_frame(
                &mut frame,
                &sampler.usages,
                available,
                ghz,
                &labels,
                celsius,
            );
        }
        if let Err(error) = output
            .write_all(frame.as_bytes())
            .and_then(|()| output.flush())
        {
            return if error.kind() == io::ErrorKind::BrokenPipe {
                Ok(())
            } else {
                Err(error)
            };
        }
        thread::sleep(SAMPLE_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::BTreeMap;

    #[test]
    fn computes_busy_delta_and_excludes_idle_and_iowait() {
        let previous = Counters([100, 20, 30, 300, 50, 10, 10, 0]);
        let current = Counters([120, 20, 40, 340, 70, 15, 15, 0]);
        assert_eq!(current.percentage_since(&previous), Some(40));
        assert_eq!(
            Counters([1, 0, 0, 2, 0, 0, 0, 0]).percentage_since(&Counters([0; 8])),
            Some(33)
        );
        assert_eq!(
            Counters([0, 0, 0, 1, 2, 0, 0, 0]).percentage_since(&Counters([0; 8])),
            Some(0)
        );
    }

    #[test]
    fn guest_time_is_not_counted_twice() {
        let mut sampler = Sampler::default();
        assert!(sampler.observe("cpu0 100 20 0 100 0 0 0 0 70 10\n"));
        assert!(sampler.observe("cpu0 130 40 0 150 0 0 0 0 95 25\n"));
        assert_eq!(
            sampler.usages,
            [Usage {
                id: 0,
                percent: Some(50)
            }]
        );
    }

    #[test]
    fn zero_delta_and_individual_counter_resets_are_unknown() {
        let previous = Counters([100; 8]);
        assert_eq!(previous.percentage_since(&previous), None);
        for index in 0..8 {
            let mut current = [200; 8];
            current[index] = 99;
            assert_eq!(Counters(current).percentage_since(&previous), None);
        }
    }

    #[test]
    fn large_deltas_do_not_overflow() {
        let zero = Counters([0; 8]);
        assert_eq!(
            Counters([u64::MAX, 0, 0, 0, 0, 0, 0, 0]).percentage_since(&zero),
            Some(100)
        );
        assert_eq!(
            Counters([u64::MAX - 1, 0, 0, 1, 0, 0, 0, 0]).percentage_since(&zero),
            Some(100)
        );
        assert_eq!(
            Counters([u64::MAX, 0, 0, 1, 0, 0, 0, 0]).percentage_since(&zero),
            None
        );
    }

    #[test]
    fn invalid_counters_remain_unknown_until_a_new_baseline() {
        for invalid in [
            "cpu3 1 2 3 4 5 6 7\n",
            "cpu3 1 2 bad 4 5 6 7 8\n",
            "cpu3 1 2 -3 4 5 6 7 8\n",
            "cpu3 18446744073709551616 2 3 4 5 6 7 8\n",
        ] {
            let mut sampler = Sampler::default();
            assert!(sampler.observe("cpu3 0 0 0 0 0 0 0 0\n"));
            assert!(sampler.observe(invalid));
            assert_eq!(sampler.previous[0].counters, None);
            assert_eq!(
                sampler.usages,
                [Usage {
                    id: 3,
                    percent: None
                }]
            );
            assert!(sampler.observe("cpu3 10 0 0 10 0 0 0 0\n"));
            assert_eq!(sampler.usages[0].percent, None);
            assert!(sampler.observe("cpu3 20 0 0 20 0 0 0 0\n"));
            assert_eq!(sampler.usages[0].percent, Some(50));
        }
    }

    #[test]
    fn sparse_ids_reordering_and_hotplug_do_not_relabel_deltas() {
        let mut sampler = Sampler::default();
        assert!(sampler.observe("cpu2 100 0 0 100 0 0 0 0\ncpu7 20 0 0 80 0 0 0 0\n"));
        assert_eq!(
            sampler.usages,
            [
                Usage {
                    id: 2,
                    percent: None
                },
                Usage {
                    id: 7,
                    percent: None
                }
            ]
        );
        assert!(sampler.observe("cpu7 40 0 0 160 0 0 0 0\ncpu2 180 0 0 120 0 0 0 0\n"));
        assert_eq!(
            sampler.usages,
            [
                Usage {
                    id: 2,
                    percent: Some(80)
                },
                Usage {
                    id: 7,
                    percent: Some(20)
                }
            ]
        );
        assert!(sampler.observe("cpu7 60 0 0 240 0 0 0 0\ncpu4 900 0 0 100 0 0 0 0\n"));
        assert_eq!(
            sampler.usages,
            [
                Usage {
                    id: 4,
                    percent: None
                },
                Usage {
                    id: 7,
                    percent: Some(20)
                }
            ]
        );
        assert!(sampler.observe("cpu2 200 0 0 200 0 0 0 0\ncpu4 950 0 0 150 0 0 0 0\n"));
        assert_eq!(
            sampler.usages,
            [
                Usage {
                    id: 2,
                    percent: None
                },
                Usage {
                    id: 4,
                    percent: Some(50)
                }
            ]
        );
    }

    #[test]
    fn read_failure_preserves_identities_but_discards_stale_deltas() {
        let mut sampler = Sampler::default();
        assert!(sampler.observe("cpu12 0 0 0 0 0 0 0 0\n"));
        assert!(sampler.observe("cpu12 1 0 0 1 0 0 0 0\n"));
        assert_eq!(sampler.usages[0].percent, Some(50));
        sampler.unavailable();
        assert_eq!(
            sampler.usages,
            [Usage {
                id: 12,
                percent: None
            }]
        );
        assert!(sampler.observe("cpu12 2 0 0 2 0 0 0 0\n"));
        assert_eq!(sampler.usages[0].percent, None);
        assert!(sampler.observe("cpu12 3 0 0 3 0 0 0 0\n"));
        assert_eq!(sampler.usages[0].percent, Some(50));
    }

    #[test]
    fn missing_or_ambiguous_cpu_rows_invalidate_the_snapshot() {
        for invalid in [
            "cpu 100 0 0 100 0 0 0 0\nintr 42\n",
            "cpu1 1 0 0 1 0 0 0 0\ncpu1 2 0 0 2 0 0 0 0\n",
        ] {
            let mut sampler = Sampler::default();
            assert!(sampler.observe("cpu1 0 0 0 0 0 0 0 0\n"));
            assert!(!sampler.observe(invalid));
            assert_eq!(
                sampler.usages,
                [Usage {
                    id: 1,
                    percent: None
                }]
            );
            assert!(sampler.observe("cpu1 3 0 0 3 0 0 0 0\n"));
            assert_eq!(sampler.usages[0].percent, None);
        }
    }

    fn rendered(usages: &[Usage], available: bool) -> BTreeMap<String, String> {
        rendered_with(usages, available, None, &[], None)
    }

    /// Renders into a reused buffer, as `main` does, and returns the record's
    /// fields. Fails unless the output is one line of the restricted JSON this
    /// renderer emits: a flat object of three distinct string fields whose
    /// values contain no quotes and no escape other than `\n`.
    fn rendered_with(
        usages: &[Usage],
        available: bool,
        ghz: Option<f32>,
        labels: &[String],
        celsius: Option<&[Option<i32>]>,
    ) -> BTreeMap<String, String> {
        let mut frame = String::from("{\"text\":\"stale\"}\n");
        render_frame(&mut frame, usages, available, ghz, labels, celsius);
        let body = frame
            .strip_prefix("{\"")
            .and_then(|rest| rest.strip_suffix("\"}\n"))
            .filter(|body| !body.contains('\n'))
            .unwrap_or_else(|| panic!("not one JSON object line: {frame:?}"));
        let mut fields = BTreeMap::new();
        for pair in body.split("\",\"") {
            let (key, value) = pair.split_once("\":\"").expect(&frame);
            let unescaped = value.replace("\\n", "");
            assert!(!unescaped.contains(['"', '\\']), "{frame:?}");
            assert!(
                fields.insert(key.to_owned(), value.to_owned()).is_none(),
                "{frame:?}"
            );
        }
        assert_eq!(
            fields.keys().collect::<Vec<_>>(),
            ["class", "text", "tooltip"]
        );
        fields
    }

    #[test]
    fn strip_shows_one_bar_per_cpu_in_tooltip_order() {
        let usage = |id, percent| Usage { id, percent };
        let record = rendered(
            &[
                usage(4, Some(100)),
                usage(9, Some(0)),
                usage(11, None),
                usage(30, Some(50)),
            ],
            true,
        );
        assert_eq!(record["text"], "█▁?▅");
        assert_eq!(
            record["tooltip"]
                .split("\\n")
                .skip(2)
                .take(4)
                .collect::<Vec<_>>(),
            ["cpu4: 100%", "cpu9: 0%", "cpu11: ?", "cpu30: 50%"]
        );
        // No row limit: every CPU stays on the bar's single line.
        let many: Vec<_> = (0..64).map(|id| usage(id, Some(25))).collect();
        assert_eq!(rendered(&many, true)["text"], "▃".repeat(64));
    }

    #[test]
    fn bars_rise_through_all_eight_levels() {
        let mut levels: Vec<_> = (0..=100).map(level).collect();
        assert_eq!((levels[0], levels[100]), ('▁', '█'));
        assert!(levels.windows(2).all(|pair| pair[0] <= pair[1]));
        levels.dedup();
        assert_eq!(levels, LEVELS);
    }

    #[test]
    fn class_marks_unknown_and_unavailable_frames() {
        let usage = |id, percent| Usage { id, percent };
        let class = |usages: &[Usage], available| rendered(usages, available)["class"].clone();
        assert_eq!(
            class(&[usage(0, Some(5)), usage(1, Some(100))], true),
            "normal"
        );
        assert_eq!(class(&[usage(0, None), usage(1, Some(5))], true), "pending");
        assert_eq!(class(&[usage(0, Some(5)), usage(1, None)], true), "pending");
        assert_eq!(class(&[usage(0, None)], false), "unavailable");
        assert_eq!(class(&[], false), "unavailable");
    }

    #[test]
    fn tooltip_leads_with_temperatures_and_marks_missing_ones() {
        let labels = ["CPU", "SSD", "Board"].map(String::from);
        let usages = [Usage {
            id: 0,
            percent: Some(5),
        }];
        let first_line = |celsius: Option<&[Option<i32>]>| {
            rendered_with(&usages, true, None, &labels, celsius)["tooltip"]
                .split("\\n")
                .next()
                .expect("split yields at least one line")
                .to_owned()
        };
        assert_eq!(
            first_line(Some(&[Some(46), None, Some(-3)][..])),
            "CPU 46°C, SSD n/a, Board -3°C"
        );
        assert_eq!(first_line(None), "CPU n/a, SSD n/a, Board n/a");
    }

    #[test]
    fn tooltip_gives_the_average_clock_or_marks_it_unknown() {
        let usages = [Usage {
            id: 0,
            percent: Some(5),
        }];
        let tooltip = |ghz| rendered_with(&usages, true, ghz, &[], None)["tooltip"].clone();
        assert_eq!(
            tooltip(Some(3.31)),
            "Average clock 3.3 GHz\\nLogical CPU usage (cell order)\\ncpu0: 5%"
        );
        assert_eq!(
            tooltip(None),
            "Average clock ?\\nLogical CPU usage (cell order)\\ncpu0: 5%"
        );
    }

    #[test]
    fn average_clock_matches_waybars_avg_frequency() {
        let cpuinfo = "processor\t: 0\ncpu MHz\t\t: 3260.527\nprocessor\t: 1\n\
                       cpu MHz\t\t: 3260.296\ncpu MHz\t\t: 3383.394\n";
        // Whole MHz 3260, 3260 and 3383: mean 3301, rounded up to 3.31 GHz.
        assert_eq!(average_ghz(cpuinfo), Some(3.31));
        // The fraction is dropped before averaging: 3.30, not 3.31.
        assert_eq!(average_ghz("cpu MHz\t\t: 3300.900\n"), Some(3.3));
        // Single precision, as Waybar shows them; f64 would give 3.4 and 3.1.
        let shown = |mhz: &str| {
            let ghz = average_ghz(&format!("cpu MHz\t\t: {mhz}\n")).expect("one cpu MHz line");
            format!("{ghz:.1}")
        };
        assert_eq!(shown("3341.000"), "3.3");
        assert_eq!(shown("3141.000"), "3.2");
        assert_eq!(average_ghz("processor\t: 0\n"), None);
        assert_eq!(average_ghz("cpu MHz\t\t: fast\n"), None);
        assert_eq!(average_ghz("cpu MHz\n"), None);
    }

    #[test]
    fn stale_or_missing_readings_are_not_current() {
        let taken = Instant::now();
        let readings = Readings {
            taken,
            celsius: vec![Some(41)],
        };
        assert_eq!(
            fresh(Some(&readings), taken + TEMPERATURE_STALE),
            Some(&[Some(41)][..])
        );
        assert_eq!(
            fresh(
                Some(&readings),
                taken + TEMPERATURE_STALE + Duration::from_secs(1)
            ),
            None
        );
        assert_eq!(fresh(None, taken), None);
    }

    #[test]
    fn a_slow_later_read_leaves_the_batch_stamped_with_its_start() {
        let start = Instant::now();
        let clock = Cell::new(start);
        // The CPU answers at once; the SSD takes longer than TEMPERATURE_STALE.
        let sources = [(46, Duration::ZERO), (41, TEMPERATURE_STALE * 2)];
        let mut celsius = Vec::new();
        let taken = read_batch(
            &sources,
            |&(value, delay)| {
                clock.set(clock.get() + delay);
                Some(value)
            },
            || clock.get(),
            &mut celsius,
        );
        assert_eq!(taken, start);
        assert_eq!(celsius, [Some(46), Some(41)]);
        // Published once the SSD answers, the batch is already too old to show.
        let readings = Readings { taken, celsius };
        assert_eq!(fresh(Some(&readings), clock.get()), None);
    }

    #[test]
    fn millidegrees_round_to_whole_degrees() {
        for (text, celsius) in [
            ("46125\n", Some(46)),
            ("46499", Some(46)),
            ("46500", Some(47)),
            ("-1500", Some(-1)),
            ("-1501", Some(-2)),
            ("", None),
            ("hot", None),
            ("9223372036854775807", None),
            ("3000000000000", None),
        ] {
            assert_eq!(parse_millidegrees(text), celsius, "{text:?}");
        }
    }

    /// A fresh, empty directory for one test.
    fn scratch(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("waybar-cores-{}-{name}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("creating a scratch directory");
        dir
    }

    #[test]
    fn hwmon_reads_the_one_hwmon_entry_of_its_device() {
        let dir = scratch("hwmon");
        let source = Hwmon {
            dir: dir.clone(),
            input: "temp1_input".into(),
        };
        assert_eq!(source.read(), None, "no hwmon entry yet");
        fs::create_dir(dir.join("device")).unwrap();
        fs::create_dir(dir.join("hwmon7")).unwrap();
        assert_eq!(source.read(), None, "no input file");
        fs::write(dir.join("hwmon7/temp1_input"), "41850\n").unwrap();
        assert_eq!(source.read(), Some(42));
        fs::create_dir(dir.join("hwmon8")).unwrap();
        assert_eq!(source.read(), None, "two candidates are a guess");
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(source.read(), None, "device gone");
    }

    #[test]
    fn arguments_name_each_source() {
        let parse = |args: &[&str]| parse_args(args.iter().map(|&arg| OsString::from(arg)));
        assert_eq!(parse(&[]), Ok(vec![]));
        assert_eq!(
            parse(&[
                "--hwmon",
                "CPU",
                "/sys/x/hwmon",
                "temp1_input",
                "--hwmon",
                "SSD (0)",
                "/sys/class/nvme/nvme0",
                "temp1_input",
            ]),
            Ok(vec![
                Sensor {
                    label: "CPU".into(),
                    source: Hwmon {
                        dir: "/sys/x/hwmon".into(),
                        input: "temp1_input".into(),
                    },
                },
                Sensor {
                    label: "SSD (0)".into(),
                    source: Hwmon {
                        dir: "/sys/class/nvme/nvme0".into(),
                        input: "temp1_input".into(),
                    },
                },
            ])
        );
        let invalid: [&[&str]; 6] = [
            &["--hwmon", "CPU", "/sys/x/hwmon"],
            &["--hwmon"],
            &["--nvidia-smi", "GPU", "/bin/nvidia-smi"],
            &["--hwmon", "", "/sys/x/hwmon", "temp1_input"],
            &["--hwmon", "C\"PU", "/sys/x/hwmon", "temp1_input"],
            &["--hwmon", "CPU\\n", "/sys/x/hwmon", "temp1_input"],
        ];
        for args in invalid {
            assert!(parse(args).is_err(), "{args:?}");
        }
    }
}
