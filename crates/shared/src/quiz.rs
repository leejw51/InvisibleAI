//! Deterministic Rust quiz generation; the web UI supplies a fresh random seed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Math,
    QuickSort,
    SlidingWindow,
    ProducerConsumer,
}
impl Kind {
    pub const ALL: [Self; 4] = [
        Self::Math,
        Self::QuickSort,
        Self::SlidingWindow,
        Self::ProducerConsumer,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Math => "Math",
            Self::QuickSort => "Quicksort",
            Self::SlidingWindow => "Sliding window",
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
        for kind in [Kind::QuickSort, Kind::SlidingWindow, Kind::ProducerConsumer] {
            let q = generate(kind, 42);
            assert!(q.code.contains("fn "));
            assert!(!q.answer.is_empty());
            assert_ne!(q.question, generate(kind, 43).question);
        }
    }
}
