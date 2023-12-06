use std::io::Read;

use base64::{engine::general_purpose, Engine};
use flate2::read::GzDecoder;

use crate::circuit::circuit_size::get_circuit_sizes;

const BYTECODE: &str = "H4sIAAAAAAAA/7VTQQ4DIQjE3bXHvgUWXfHWr9TU/f8TmrY2Ma43cRJCwmEYBrAAYOGKteRHyYyHcznsmZieuMckHp1Ph5CQF//ahTmLkxBTDBjJcabTRz7xB1Nx4RhoUdS16un6cpmOl6bxEsdAmpprvVuJD5bOLdwmzAJNn9a/e6em2nzGcrYJvBb0jn7W3FZ/R1hRXjSP+mBB/5FMpbN+oj/eG6c6pXEFAAA=";

#[test]
fn test_circuit_size_method() {
    let acir_buffer = general_purpose::STANDARD.decode(BYTECODE).unwrap();
    let mut decoder = GzDecoder::new(acir_buffer.as_slice());
    let mut acir_buffer_uncompressed = Vec::<u8>::new();
    decoder.read_to_end(&mut acir_buffer_uncompressed).unwrap();

    let sizes = get_circuit_sizes(&acir_buffer_uncompressed).unwrap();
    assert_eq!(sizes.exact, 5);
    assert_eq!(sizes.subgroup, 16);
    assert_eq!(sizes.total, 10);
}
