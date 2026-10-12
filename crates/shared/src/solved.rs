//! Whether an answer solved the arithmetic problem on a captured screen.
//! The check is ours, not the model's: the problem is read from the screen
//! text, computed here, and the answer counts only when it states the result.

#[derive(Clone, Debug, PartialEq)]
pub struct Solved {
    /// The problem as written on the screen, e.g. `98 - 4`.
    pub problem: String,
    /// The computed result, formatted as the answer should state it.
    pub result: String,
}

/// The first arithmetic problem on the screen (`98 - 4 = ?`, `7 × 5 =`) that
/// the answer gets right, if any.
pub fn check(screen: &str, answer: &str) -> Option<Solved> {
    let stated = numbers(answer);
    screen
        .lines()
        .filter_map(problem)
        .find_map(|(problem, value)| {
            let result = format_number(value);
            stated
                .iter()
                .any(|n| (n - value).abs() < 1e-9)
                .then_some(Solved { problem, result })
        })
}

/// Every arithmetic problem the text asks for, each with its exact result.
pub fn problems(text: &str) -> Vec<Solved> {
    let mut out: Vec<Solved> = vec![];
    for (problem, value) in text.lines().filter_map(problem) {
        if !out.iter().any(|p| p.problem == problem) {
            out.push(Solved {
                problem,
                result: format_number(value),
            });
        }
    }
    out
}

/// A line that asks for a result: two or more numbers joined by operators,
/// then `=` with nothing, `?`, `□`, or `_` after it.
fn problem(line: &str) -> Option<(String, f64)> {
    let (left, right) = line.split_once('=')?;
    let right = right.trim();
    if !(right.is_empty()
        || right
            .chars()
            .all(|c| matches!(c, '?' | '？' | '□' | '_' | ' ')))
    {
        return None;
    }
    // Keep the expression's own tail: OCR often joins it with words before it.
    let start = left
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_ascii_digit() || " .,+-−–*×x÷/()".contains(*c))
        .last()
        .map(|(i, _)| i)?;
    // Labels such as "Q1." can also survive in that tail; drop words from the
    // front until what remains is a whole expression.
    let tail = &left[start..];
    let starts = std::iter::once(0).chain(tail.match_indices(' ').map(|(i, _)| i + 1));
    starts.into_iter().find_map(|from| {
        let expression = tail[from..].trim();
        let operators = expression
            .chars()
            .filter(|c| "+-−–*×x÷/".contains(*c))
            .count();
        let value = evaluate(expression)?;
        (operators > 0 && value.is_finite()).then(|| (expression.to_owned(), value))
    })
}

#[derive(Clone, Copy, PartialEq)]
enum Token {
    Number(f64),
    Op(char),
    Open,
    Close,
}

fn tokens(expression: &str) -> Option<Vec<Token>> {
    let mut out = vec![];
    let mut chars = expression.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ' ' => {}
            '0'..='9' => {
                let mut digits = c.to_string();
                while let Some(&d) = chars.peek() {
                    if d.is_ascii_digit() || d == '.' {
                        digits.push(d);
                    } else if d == ',' {
                        // Thousands separators only; a comma is never a decimal here.
                    } else {
                        break;
                    }
                    chars.next();
                }
                out.push(Token::Number(digits.parse().ok()?));
            }
            '+' => out.push(Token::Op('+')),
            '-' | '−' | '–' => out.push(Token::Op('-')),
            '*' | '×' | 'x' => out.push(Token::Op('*')),
            '÷' | '/' => out.push(Token::Op('/')),
            '(' => out.push(Token::Open),
            ')' => out.push(Token::Close),
            _ => return None,
        }
    }
    Some(out)
}

fn evaluate(expression: &str) -> Option<f64> {
    let tokens = tokens(expression)?;
    let mut at = 0;
    let value = sum(&tokens, &mut at)?;
    (at == tokens.len()).then_some(value)
}
fn sum(t: &[Token], at: &mut usize) -> Option<f64> {
    let mut value = product(t, at)?;
    while let Some(Token::Op(op @ ('+' | '-'))) = t.get(*at) {
        *at += 1;
        let next = product(t, at)?;
        value = if *op == '+' {
            value + next
        } else {
            value - next
        };
    }
    Some(value)
}
fn product(t: &[Token], at: &mut usize) -> Option<f64> {
    let mut value = atom(t, at)?;
    while let Some(Token::Op(op @ ('*' | '/'))) = t.get(*at) {
        *at += 1;
        let next = atom(t, at)?;
        value = if *op == '*' {
            value * next
        } else if next == 0.0 {
            return None;
        } else {
            value / next
        };
    }
    Some(value)
}
fn atom(t: &[Token], at: &mut usize) -> Option<f64> {
    match *t.get(*at)? {
        Token::Number(n) => {
            *at += 1;
            Some(n)
        }
        Token::Op('-') => {
            *at += 1;
            Some(-atom(t, at)?)
        }
        Token::Open => {
            *at += 1;
            let value = sum(t, at)?;
            (t.get(*at) == Some(&Token::Close)).then(|| {
                *at += 1;
                value
            })
        }
        _ => None,
    }
}

/// Every number the answer states, so `94` in "The answer is **94**." counts.
fn numbers(text: &str) -> Vec<f64> {
    let mut out = vec![];
    let mut current = String::new();
    let mut flush = |current: &mut String| {
        let n = current.trim_end_matches('.').replace(',', "");
        if let Ok(v) = n.parse::<f64>() {
            out.push(v);
        }
        current.clear();
    };
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let negative = c == '-'
            && current.is_empty()
            && chars.get(i + 1).is_some_and(char::is_ascii_digit)
            && !chars
                .get(i.wrapping_sub(1))
                .is_some_and(|p| p.is_alphanumeric());
        if c.is_ascii_digit() || negative || (!current.is_empty() && matches!(c, '.' | ',')) {
            current.push(c);
        } else {
            flush(&mut current);
        }
    }
    flush(&mut current);
    out
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        let s = format!("{value:.4}");
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_right_answer_on_a_busy_screen_is_solved() {
        let screen = "Invisible AI · Chat\nLocal server 127.0.0.1:3000\nStatus: 721\n98 - 4 = ?\nID: b91b89e9";
        assert_eq!(
            check(screen, "94\nWorking: 98 - 4 = 94."),
            Some(Solved {
                problem: "98 - 4".into(),
                result: "94".into()
            })
        );
    }
    #[test]
    fn problems_lists_each_question_once_with_its_result() {
        let found = problems("Quiz\n98 - 4 = ?\nStatus 721\n6 × 7 =\n98 - 4 = ?");
        assert_eq!(
            found
                .iter()
                .map(|p| format!("{} = {}", p.problem, p.result))
                .collect::<Vec<_>>(),
            ["98 - 4 = 94", "6 × 7 = 42"]
        );
    }
    #[test]
    fn a_wrong_answer_is_not_solved() {
        assert_eq!(check("98 - 4 = ?", "The answer is 93."), None);
        assert_eq!(check("What is 7 + 5 =", "It is 11"), None);
    }
    #[test]
    fn operators_precedence_and_ocr_spellings() {
        assert_eq!(check("Q1. 7 + 5 × 2 =", "17").unwrap().problem, "7 + 5 × 2");
        assert_eq!(check("12 ÷ 4 = ?", "**3**").unwrap().result, "3");
        assert_eq!(check("(3 + 4) x 2 = □", "14").unwrap().result, "14");
        assert_eq!(check("1,200 − 200 = ?", "1,000").unwrap().result, "1000");
        assert_eq!(check("7 / 2 = ?", "3.5").unwrap().result, "3.5");
        assert_eq!(check("3 - 10 = ?", "-7").unwrap().result, "-7");
    }
    #[test]
    fn plain_text_and_finished_equations_are_not_problems() {
        assert_eq!(check("Status: 721 · 127.0.0.1", "721"), None);
        assert_eq!(check("98 - 4 = 94", "94"), None);
        assert_eq!(check("version = ?", "1"), None);
        assert_eq!(check("5 / 0 = ?", "0"), None);
    }
}
