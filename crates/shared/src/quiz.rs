//! Deterministic Rust quiz generation; the web UI supplies a fresh random seed.
//! Every kind carries dynamic values, so a page that regenerates its quiz on a
//! timer keeps testing the AI on numbers it has not seen before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Math,
    Sequence,
    WordProblem,
    QuickSort,
    SlidingWindow,
    BinarySearch,
    TwoSum,
    PredictOutput,
    ProducerConsumer,
}
impl Kind {
    pub const ALL: [Self; 9] = [
        Self::Math,
        Self::Sequence,
        Self::WordProblem,
        Self::QuickSort,
        Self::SlidingWindow,
        Self::BinarySearch,
        Self::TwoSum,
        Self::PredictOutput,
        Self::ProducerConsumer,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Math => "Math",
            Self::Sequence => "Sequence",
            Self::WordProblem => "Word problem",
            Self::QuickSort => "Quicksort",
            Self::SlidingWindow => "Sliding window",
            Self::BinarySearch => "Binary search",
            Self::TwoSum => "Two sum",
            Self::PredictOutput => "Predict the output",
            Self::ProducerConsumer => "Producer / consumer",
        }
    }
}
#[derive(Clone, PartialEq)]
pub struct Quiz {
    pub kind: Kind,
    pub title: String,
    pub question: String,
    pub code: String,
    pub answer: String,
}
struct Rng(u64);
impl Rng {
    fn number(&mut self, low: i32, high: i32) -> i32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        low + ((self.0 >> 32) % ((high - low + 1) as u64)) as i32
    }
}
pub fn generate(kind: Kind, seed: u64) -> Quiz {
    let mut rng = Rng(seed);
    let (title, question, code, answer) = match kind {
        Kind::Math => {
            let a = rng.number(2, 99);
            let b = rng.number(2, 30);
            let (question, answer) = match rng.number(0, 3) {
                0 => (format!("{a} + {b} = ?"), (a + b).to_string()),
                1 => (format!("{} − {b} = ?", a + b), a.to_string()),
                2 => (format!("{a} × {b} = ?"), (a * b).to_string()),
                _ => (format!("{} ÷ {b} = ?", a * b), a.to_string()),
            };
            (
                "A fresh little calculation.".into(),
                question,
                String::new(),
                answer,
            )
        }
        Kind::Sequence => {
            let (terms, next, rule) = match rng.number(0, 3) {
                0 => {
                    let start = rng.number(1, 40);
                    let step = rng.number(3, 17);
                    let terms = (0..5).map(|i| start + step * i).collect::<Vec<_>>();
                    (terms, start + step * 5, format!("add {step} each time"))
                }
                1 => {
                    let start = rng.number(1, 6);
                    let ratio = rng.number(2, 4);
                    let mut terms = vec![start];
                    for _ in 0..4 {
                        terms.push(terms.last().unwrap() * ratio);
                    }
                    let next = terms.last().unwrap() * ratio;
                    (terms, next, format!("multiply by {ratio} each time"))
                }
                2 => {
                    let offset = rng.number(-5, 9);
                    let first = rng.number(1, 4);
                    let terms = (first..first + 5)
                        .map(|n| n * n + offset)
                        .collect::<Vec<_>>();
                    let n = first + 5;
                    (
                        terms,
                        n * n + offset,
                        format!("n² {} {}", if offset < 0 { "−" } else { "+" }, offset.abs()),
                    )
                }
                _ => {
                    let mut terms = vec![rng.number(1, 9), rng.number(1, 9)];
                    for _ in 0..3 {
                        let len = terms.len();
                        terms.push(terms[len - 1] + terms[len - 2]);
                    }
                    let len = terms.len();
                    (
                        terms.clone(),
                        terms[len - 1] + terms[len - 2],
                        "each term is the sum of the two before it".to_string(),
                    )
                }
            };
            let shown = terms
                .iter()
                .map(|t| t.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            (
                "Spot the pattern.".into(),
                format!("{shown}, … What is the next number? Explain the rule."),
                String::new(),
                format!("Next: {next}\nRule: {rule}."),
            )
        }
        Kind::WordProblem => {
            let (question, answer) = match rng.number(0, 2) {
                0 => {
                    let fill = rng.number(6, 20);
                    let drain = rng.number(1, fill - 2);
                    let minutes = rng.number(4, 15);
                    let start = rng.number(10, 80);
                    let capacity = start + (fill - drain) * minutes;
                    (
                        format!(
                            "A tank holds {capacity} liters and already has {start} liters in it. A pipe fills it at {fill} liters per minute while a drain lets out {drain} liters per minute. With both open, how many minutes until the tank is full?"
                        ),
                        format!(
                            "{minutes} minutes\nNet fill: {fill} − {drain} = {} L/min. Missing: {capacity} − {start} = {} L. {} ÷ {} = {minutes}.",
                            fill - drain,
                            capacity - start,
                            capacity - start,
                            fill - drain
                        ),
                    )
                }
                1 => {
                    let price = rng.number(3, 25);
                    let count = rng.number(4, 12);
                    let discount = rng.number(2, 9);
                    let paid = rng.number(50, 200).max(price * count - discount + 1);
                    let cost = price * count - discount;
                    (
                        format!(
                            "Tickets cost {price} each. Someone buys {count} tickets, uses a {discount} coupon, and pays with {paid}. How much change do they get?"
                        ),
                        format!(
                            "{} change\nCost: {price} × {count} − {discount} = {cost}. Change: {paid} − {cost} = {}.",
                            paid - cost,
                            paid - cost
                        ),
                    )
                }
                _ => {
                    let speed = rng.number(40, 90);
                    let hours = rng.number(2, 6);
                    let stop = rng.number(1, 4) * 15;
                    let distance = speed * hours;
                    (
                        format!(
                            "A car drives {distance} km at a steady {speed} km/h, then stops for {stop} minutes. How many minutes does the whole trip take?"
                        ),
                        format!(
                            "{} minutes\nDriving: {distance} ÷ {speed} = {hours} h = {} min. Plus the {stop} min stop.",
                            hours * 60 + stop,
                            hours * 60
                        ),
                    )
                }
            };
            ("Reason it through.".into(), question, String::new(), answer)
        }
        Kind::QuickSort => {
            let values = (0..8).map(|_| rng.number(-20, 99)).collect::<Vec<_>>();
            let mut sorted = values.clone();
            sorted.sort();
            (
                "Sort it out in Rust.".into(),
                format!(
                    "Implement quicksort for {values:?}. Sort in ascending order without calling sort(). Handle duplicates and empty slices. Explain average and worst-case time complexity."
                ),
                "fn quicksort(values: &mut [i32]) {\n    // Partition, then sort each side.\n}"
                    .into(),
                format!("Sorted values: {sorted:?}\nAverage: O(n log n). Worst case: O(n²)."),
            )
        }
        Kind::SlidingWindow => {
            let values = (0..9).map(|_| rng.number(-8, 30)).collect::<Vec<_>>();
            let k = rng.number(2, 4) as usize;
            let best = values
                .windows(k)
                .map(|w| w.iter().sum::<i32>())
                .max()
                .unwrap();
            ("Find the best window.".into(),format!("For {values:?}, find the maximum sum of {k} consecutive values. Write a Rust sliding-window solution in O(n) time and O(1) extra space. Return None for k = 0 or k greater than the slice length."),"fn max_window_sum(values: &[i32], k: usize) -> Option<i32> {\n    // Move the window without recomputing its sum.\n    todo!()\n}".into(),format!("Maximum window sum: {best}\nAdd the incoming value and subtract the outgoing value on each step."))
        }
        Kind::BinarySearch => {
            let mut values = vec![rng.number(-30, 10)];
            for _ in 0..8 {
                let gap = rng.number(2, 11);
                values.push(values.last().unwrap() + gap);
            }
            let present = rng.number(0, 2) != 0;
            let target = if present {
                values[rng.number(0, 8) as usize]
            } else {
                let i = rng.number(0, 7) as usize;
                values[i] + 1
            };
            let found = values.iter().position(|&v| v == target);
            let answer = match found {
                Some(i) => format!(
                    "Some({i})\n{target} sits at index {i}. Keep lo and hi as a half-open range; stop when they meet."
                ),
                None => format!(
                    "None\n{target} is not in the slice. Keep lo and hi as a half-open range; stop when they meet."
                ),
            };
            (
                "Find it fast.".into(),
                format!("Implement binary search over the sorted slice {values:?}. Return the index of {target} as Option<usize>, or None when it is absent. What does your function return for {target}?"),
                "fn binary_search(values: &[i32], target: i32) -> Option<usize> {\n    // Halve the range on every step.\n    todo!()\n}".into(),
                answer,
            )
        }
        Kind::TwoSum => {
            let mut values = Vec::new();
            while values.len() < 8 {
                let v = rng.number(-15, 40);
                if !values.contains(&v) {
                    values.push(v);
                }
            }
            let i = rng.number(0, 7) as usize;
            let mut j = rng.number(0, 7) as usize;
            if j == i {
                j = (i + 1) % 8;
            }
            let target = values[i] + values[j];
            let mut pairs = Vec::new();
            for a in 0..values.len() {
                for b in a + 1..values.len() {
                    if values[a] + values[b] == target {
                        pairs.push(format!("({a}, {b}) → {} + {}", values[a], values[b]));
                    }
                }
            }
            (
                "Pair them up.".into(),
                format!("For {values:?}, find two different indices whose values add up to {target}. Write a Rust solution in O(n) time using a HashMap, returning Option<(usize, usize)>. Which indices does it find?"),
                "use std::collections::HashMap;\n\nfn two_sum(values: &[i32], target: i32) -> Option<(usize, usize)> {\n    // Remember each value's index as you go.\n    todo!()\n}".into(),
                format!("Valid pairs: {}\nAny one of them is correct.", pairs.join("; ")),
            )
        }
        Kind::PredictOutput => {
            let (question, code, output, why) = if rng.number(0, 1) == 0 {
                let limit = rng.number(12, 40);
                let a = rng.number(2, 5);
                let b = rng.number(a + 1, 9);
                let mut total = 0;
                let mut hits = 0;
                for n in 1..=limit {
                    if n % a == 0 && n % b == 0 {
                        total -= n;
                    } else if n % a == 0 || n % b == 0 {
                        total += n;
                        hits += 1;
                    }
                }
                (
                    format!(
                        "This program walks the numbers 1 to {limit}. Predict exactly what it prints without running it, and explain how you got there."
                    ),
                    format!(
                        "fn main() {{\n    let mut total = 0;\n    let mut hits = 0;\n    for n in 1..={limit} {{\n        if n % {a} == 0 && n % {b} == 0 {{\n            total -= n;\n        }} else if n % {a} == 0 || n % {b} == 0 {{\n            total += n;\n            hits += 1;\n        }}\n    }}\n    println!(\"{{hits}} {{total}}\");\n}}"
                    ),
                    format!("{hits} {total}"),
                    format!(
                        "Multiples of {a} or {b} up to {limit} are added, but a multiple of both is subtracted and not counted."
                    ),
                )
            } else {
                let start = rng.number(6, 400);
                let mut n = start;
                let mut steps = 0;
                let mut peak = n;
                while n != 1 {
                    n = if n % 2 == 0 { n / 2 } else { 3 * n + 1 };
                    steps += 1;
                    peak = peak.max(n);
                }
                (
                    format!(
                        "This program starts from {start}. Predict exactly what it prints without running it, and explain how you got there."
                    ),
                    format!(
                        "fn main() {{\n    let mut n: u64 = {start};\n    let mut steps = 0;\n    let mut peak = n;\n    while n != 1 {{\n        n = if n % 2 == 0 {{ n / 2 }} else {{ 3 * n + 1 }};\n        steps += 1;\n        peak = peak.max(n);\n    }}\n    println!(\"{{steps}} {{peak}}\");\n}}"
                    ),
                    format!("{steps} {peak}"),
                    format!(
                        "The Collatz sequence from {start} reaches 1 after {steps} steps; its largest value is {peak}."
                    ),
                )
            };
            (
                "What does it print?".into(),
                question,
                code,
                format!("{output}\n{why}"),
            )
        }
        Kind::ProducerConsumer => {
            let producers = rng.number(2, 4);
            let items = rng.number(3, 9);
            let capacity = rng.number(1, 4);
            let count = producers * items;
            let sum = count * (count - 1) / 2;
            (
                "Make the threads work together.".into(),
                format!(
                    "Write a Rust producer/consumer program using sync_channel({capacity}). Spawn {producers} producers. Producer p sends p * {items} + i for i in 0..{items}. One consumer receives every item exactly once, prints the count and sum, then exits. Join the threads and avoid deadlock."
                ),
                format!(
                    "use std::sync::mpsc::sync_channel;\nuse std::thread;\n\nfn main() {{\n    let (tx, rx) = sync_channel::<i32>({capacity});\n    // Spawn producers and a consumer.\n    // Close the channel and join every thread.\n}}"
                ),
                format!(
                    "Count: {count}\nSum: {sum}\nArrival order may vary. Drop the original sender after spawning producers so the consumer can finish."
                ),
            )
        }
    };
    Quiz {
        kind,
        title,
        question,
        code,
        answer,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seeds_change_numbers_and_math_answers_are_correct() {
        assert_ne!(
            generate(Kind::Math, 1).question,
            generate(Kind::Math, 2).question
        );
        for seed in 0..100 {
            let q = generate(Kind::Math, seed);
            let parts = q.question.split_whitespace().collect::<Vec<_>>();
            let a = parts[0].parse::<i32>().unwrap();
            let b = parts[2].parse::<i32>().unwrap();
            let expected = match parts[1] {
                "+" => a + b,
                "−" => a - b,
                "×" => a * b,
                _ => a / b,
            };
            assert_eq!(q.answer, expected.to_string());
        }
    }
    #[test]
    fn coding_quizzes_have_rust_scaffolds_and_expected_results() {
        for kind in [
            Kind::QuickSort,
            Kind::SlidingWindow,
            Kind::BinarySearch,
            Kind::TwoSum,
            Kind::PredictOutput,
            Kind::ProducerConsumer,
        ] {
            let q = generate(kind, 42);
            assert!(q.code.contains("fn "), "{kind:?}");
            assert!(!q.answer.is_empty());
            assert_ne!(q.question, generate(kind, 43).question);
        }
    }
    #[test]
    fn every_kind_changes_with_the_seed_and_has_an_answer() {
        for kind in Kind::ALL {
            let questions = (0..40)
                .map(|seed| generate(kind, seed))
                .inspect(|q| assert!(!q.answer.is_empty(), "{kind:?}"))
                .map(|q| q.question + &q.code)
                .collect::<std::collections::HashSet<_>>();
            assert!(
                questions.len() > 30,
                "{kind:?} repeats: {}",
                questions.len()
            );
        }
    }
    fn numbers(text: &str) -> Vec<i64> {
        text.split(|c: char| !(c.is_ascii_digit() || c == '-'))
            .filter_map(|p| p.parse().ok())
            .collect()
    }
    #[test]
    fn sequence_answers_follow_the_shown_terms() {
        for seed in 0..200 {
            let q = generate(Kind::Sequence, seed);
            let terms = numbers(q.question.split(", …").next().unwrap());
            let next = numbers(q.answer.lines().next().unwrap())[0];
            assert_eq!(terms.len(), 5, "{}", q.question);
            let gaps = terms.windows(2).map(|w| w[1] - w[0]).collect::<Vec<_>>();
            let arithmetic = gaps.iter().all(|&g| g == gaps[0]) && next == terms[4] + gaps[0];
            let geometric = terms[0] != 0
                && terms[1] % terms[0] == 0
                && next == terms[4] * (terms[1] / terms[0]);
            let squares =
                gaps.windows(2).all(|w| w[1] - w[0] == 2) && next - terms[4] == gaps[3] + 2;
            let fibonacci = next == terms[4] + terms[3];
            assert!(
                arithmetic || geometric || squares || fibonacci,
                "{} -> {}",
                q.question,
                q.answer
            );
        }
    }
    #[test]
    fn word_problems_compute_their_own_numbers() {
        for seed in 0..200 {
            let q = generate(Kind::WordProblem, seed);
            let n = numbers(&q.question);
            let got = numbers(q.answer.lines().next().unwrap())[0];
            let expected = if q.question.starts_with("A tank") {
                (n[0] - n[1]) / (n[2] - n[3])
            } else if q.question.starts_with("Tickets") {
                n[3] - (n[0] * n[1] - n[2])
            } else {
                n[0] / n[1] * 60 + n[2]
            };
            assert_eq!(got, expected, "{}", q.question);
        }
    }
    #[test]
    fn search_quizzes_report_true_positions() {
        for seed in 0..200 {
            let q = generate(Kind::BinarySearch, seed);
            let list = q
                .question
                .split('[')
                .nth(1)
                .unwrap()
                .split(']')
                .next()
                .unwrap();
            let values = numbers(list);
            assert!(values.windows(2).all(|w| w[0] < w[1]));
            let target = numbers(q.question.split("index of ").nth(1).unwrap())[0];
            let expected = values.iter().position(|&v| v == target);
            let first = q.answer.lines().next().unwrap();
            match expected {
                Some(i) => assert_eq!(first, format!("Some({i})")),
                None => assert_eq!(first, "None"),
            }
            let q = generate(Kind::TwoSum, seed);
            let list = q
                .question
                .split('[')
                .nth(1)
                .unwrap()
                .split(']')
                .next()
                .unwrap();
            let values = numbers(list);
            let target = numbers(q.question.split("add up to ").nth(1).unwrap())[0];
            let pairs = q.answer.lines().next().unwrap();
            assert!(pairs.contains('('));
            for pair in pairs.trim_start_matches("Valid pairs: ").split("; ") {
                let idx = numbers(pair.split(" → ").next().unwrap());
                assert_eq!(
                    values[idx[0] as usize] + values[idx[1] as usize],
                    target,
                    "{pair}"
                );
            }
        }
    }
    #[test]
    fn predicted_output_matches_the_shown_program() {
        for seed in 0..200 {
            let q = generate(Kind::PredictOutput, seed);
            let first = q.answer.lines().next().unwrap();
            if q.code.contains("hits") {
                // The code reads `1..={limit} ... n % {a} == 0 && n % {b} == 0`.
                let n = numbers(q.code.split("1..=").nth(1).unwrap());
                let (limit, a, b) = (n[0], n[1], n[3]);
                let (mut total, mut hits) = (0, 0);
                for v in 1..=limit {
                    if v % a == 0 && v % b == 0 {
                        total -= v;
                    } else if v % a == 0 || v % b == 0 {
                        total += v;
                        hits += 1;
                    }
                }
                assert_eq!(first, format!("{hits} {total}"), "{}", q.code);
            } else {
                let mut v = numbers(q.code.split("u64 = ").nth(1).unwrap())[0] as u64;
                let (mut steps, mut peak) = (0, v);
                while v != 1 {
                    v = if v.is_multiple_of(2) {
                        v / 2
                    } else {
                        3 * v + 1
                    };
                    steps += 1;
                    peak = peak.max(v);
                }
                assert_eq!(first, format!("{steps} {peak}"), "{}", q.code);
            }
        }
    }
}
