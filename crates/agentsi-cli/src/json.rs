//! `JSON.stringify(value, null, 2)` for a typed value. The keys keep the order
//! of the struct, and a number prints as JavaScript prints it: `1` for `1.0`.

use serde::Serialize;
use serde_json::ser::{Formatter, PrettyFormatter};
use std::io;

struct Js(PrettyFormatter<'static>);

/// A float as `Number.prototype.toString` prints it, in the range that costs and rates use.
fn js_float(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        // Also turns -0 into 0.
        return format!("{}", v as i64);
    }
    if v.abs() >= 1e-6 && v.abs() < 1e21 {
        return format!("{v}");
    }
    serde_json::to_string(&v).unwrap_or_default()
}

impl Formatter for Js {
    fn write_f64<W: ?Sized + io::Write>(&mut self, w: &mut W, v: f64) -> io::Result<()> {
        w.write_all(js_float(v).as_bytes())
    }
    fn begin_array<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.begin_array(w)
    }
    fn end_array<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.end_array(w)
    }
    fn begin_array_value<W: ?Sized + io::Write>(
        &mut self,
        w: &mut W,
        first: bool,
    ) -> io::Result<()> {
        self.0.begin_array_value(w, first)
    }
    fn end_array_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.end_array_value(w)
    }
    fn begin_object<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.begin_object(w)
    }
    fn end_object<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.end_object(w)
    }
    fn begin_object_key<W: ?Sized + io::Write>(
        &mut self,
        w: &mut W,
        first: bool,
    ) -> io::Result<()> {
        self.0.begin_object_key(w, first)
    }
    fn begin_object_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.begin_object_value(w)
    }
    fn end_object_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.end_object_value(w)
    }
}

pub fn pretty<T: Serialize>(value: &T) -> String {
    let mut out = Vec::new();
    let mut ser =
        serde_json::Serializer::with_formatter(&mut out, Js(PrettyFormatter::with_indent(b"  ")));
    value.serialize(&mut ser).expect("a typed value serializes");
    String::from_utf8(out).expect("JSON is UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn it_prints_like_json_stringify() {
        #[derive(Serialize)]
        struct T {
            zebra: f64,
            apple: f64,
            none: Option<u8>,
            empty: Vec<u8>,
            list: Vec<&'static str>,
        }
        let t = T {
            zebra: 1.0,
            apple: 0.3333333333333333,
            none: None,
            empty: vec![],
            list: vec!["a", "é"],
        };
        assert_eq!(
            pretty(&t),
            "{\n  \"zebra\": 1,\n  \"apple\": 0.3333333333333333,\n  \"none\": null,\n  \"empty\": [],\n  \"list\": [\n    \"a\",\n    \"é\"\n  ]\n}"
        );
        assert_eq!(pretty(&json!({})), "{}");
    }

    #[test]
    fn a_float_prints_as_a_javascript_number() {
        assert_eq!(js_float(0.0), "0");
        assert_eq!(js_float(-0.0), "0");
        assert_eq!(js_float(2.5), "2.5");
        assert_eq!(js_float(0.000002), "0.000002");
        assert_eq!(js_float(1e-7), "1e-7");
        assert_eq!(js_float(120.0), "120");
    }
}
