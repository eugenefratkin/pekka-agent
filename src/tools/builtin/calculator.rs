use crate::error::ToolError;
use crate::tools::Tool;
use async_trait::async_trait;
use serde_json::json;
use tokio_util::sync::CancellationToken;

/// Simple arithmetic evaluator supporting `+`, `-`, `*`, `/`, and `(`.
pub struct CalculatorTool;

#[async_trait]
impl Tool for CalculatorTool {
    fn name(&self) -> &str {
        "calculator"
    }

    fn description(&self) -> &str {
        "Evaluates simple arithmetic expressions, e.g. '2 + 3 * 4' or '(10 / 2) - 1'"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "expression": {
                    "type": "string",
                    "description": "The arithmetic expression to evaluate"
                }
            },
            "required": ["expression"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _cancel: CancellationToken,
    ) -> Result<String, ToolError> {
        let expr = params
            .get("expression")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("missing 'expression' field".into()))?;

        let result = evaluate(expr)
            .map_err(|e| ToolError::ExecutionFailed(e))?;

        Ok(result.to_string())
    }
}

// ── Minimal expression evaluator ─────────────────────────────────────────────
// Implements recursive descent: expr → term (('+' | '-') term)*
//                               term → factor (('*' | '/') factor)*
//                               factor → '(' expr ')' | number

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Self { input: s.as_bytes(), pos: 0 }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.input.len() && self.input[self.pos] == b' ' {
            self.pos += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.input.get(self.pos).copied()
    }

    fn consume(&mut self) -> u8 {
        let b = self.input[self.pos];
        self.pos += 1;
        b
    }

    fn parse_number(&mut self) -> Result<f64, String> {
        self.skip_ws();
        let start = self.pos;
        if self.pos < self.input.len() && self.input[self.pos] == b'-' {
            self.pos += 1;
        }
        while self.pos < self.input.len()
            && (self.input[self.pos].is_ascii_digit() || self.input[self.pos] == b'.')
        {
            self.pos += 1;
        }
        let s = std::str::from_utf8(&self.input[start..self.pos])
            .map_err(|e| e.to_string())?;
        s.parse::<f64>().map_err(|e| e.to_string())
    }

    fn parse_factor(&mut self) -> Result<f64, String> {
        if self.peek() == Some(b'(') {
            self.consume(); // '('
            let val = self.parse_expr()?;
            self.skip_ws();
            if self.peek() != Some(b')') {
                return Err("expected ')'".into());
            }
            self.consume();
            Ok(val)
        } else {
            self.parse_number()
        }
    }

    fn parse_term(&mut self) -> Result<f64, String> {
        let mut val = self.parse_factor()?;
        loop {
            match self.peek() {
                Some(b'*') => { self.consume(); val *= self.parse_factor()?; }
                Some(b'/') => {
                    self.consume();
                    let d = self.parse_factor()?;
                    if d == 0.0 { return Err("division by zero".into()); }
                    val /= d;
                }
                _ => break,
            }
        }
        Ok(val)
    }

    fn parse_expr(&mut self) -> Result<f64, String> {
        let mut val = self.parse_term()?;
        loop {
            match self.peek() {
                Some(b'+') => { self.consume(); val += self.parse_term()?; }
                Some(b'-') => { self.consume(); val -= self.parse_term()?; }
                _ => break,
            }
        }
        Ok(val)
    }
}

fn evaluate(expr: &str) -> Result<f64, String> {
    let mut p = Parser::new(expr);
    let val = p.parse_expr()?;
    p.skip_ws();
    if p.pos != p.input.len() {
        return Err(format!("unexpected char '{}' at pos {}", p.input[p.pos] as char, p.pos));
    }
    Ok(val)
}

#[cfg(test)]
mod tests {
    use super::evaluate;

    #[test]
    fn basic_arithmetic() {
        assert_eq!(evaluate("2 + 3").unwrap(), 5.0);
        assert_eq!(evaluate("10 / 2").unwrap(), 5.0);
        assert_eq!(evaluate("(2 + 3) * 4").unwrap(), 20.0);
    }
}
