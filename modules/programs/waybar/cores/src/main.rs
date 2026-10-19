use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, Read, Write};
use std::thread;
use std::time::Duration;

const SAMPLE_INTERVAL: Duration = Duration::from_secs(2);

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

fn render_frame(frame: &mut String, usages: &[Usage], available: bool) {
    frame.clear();
    // JSON contains only these fixed strings, bar glyphs, parsed numeric CPU
    // IDs and computed percentages. Tooltip line breaks are escaped explicitly.
    frame.push_str("{\"text\":\"");
    if usages.is_empty() {
        frame.push_str("cores unavailable");
    }
    for usage in usages {
        frame.push(usage.percent.map_or('?', level));
    }
    frame.push_str("\",\"tooltip\":\"");
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
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let mut input = String::new();
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
        render_frame(&mut frame, &sampler.usages, available);
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

    /// Renders into a reused buffer, as `main` does, and returns the record's
    /// fields. Fails unless the output is one line of the restricted JSON this
    /// renderer emits: a flat object of three distinct string fields whose
    /// values contain no quotes and no escape other than `\n`.
    fn rendered(usages: &[Usage], available: bool) -> BTreeMap<String, String> {
        let mut frame = String::from("{\"text\":\"stale\"}\n");
        render_frame(&mut frame, usages, available);
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
                .skip(1)
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
}
