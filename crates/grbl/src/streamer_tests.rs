use crate::Streamer;
use crate::proto_tests::Rng;

#[test]
fn documented_example() {
    // The Interface page's example: 25 + 40 + 31 bytes fit (96), a 58-byte line waits until the
    // first two are answered.
    let mut s = Streamer::new(128);
    assert!(s.push(24));
    assert!(s.push(39));
    assert!(s.push(30));
    assert_eq!(s.used(), 96);
    assert!(!s.fits(57));
    assert_eq!(s.ack(), Some(25));
    assert!(!s.push(57), "71 + 58 = 129 > 128");
    assert_eq!(s.ack(), Some(40));
    assert!(s.push(57));
    assert!(s.push(19));
    assert_eq!(s.used(), 31 + 58 + 20);
    assert_eq!(s.in_flight(), 3);
    s.clear();
    assert!(s.is_empty());
    assert_eq!(s.ack(), None);
}

#[test]
fn exactly_full_and_too_long() {
    let mut s = Streamer::new(128);
    assert!(s.push(127));
    assert_eq!(s.used(), 128);
    assert!(!s.push(0));
    s.ack();
    assert!(!s.push(128), "a line can never exceed the buffer");
    assert!(!s.push(usize::MAX));
}

#[test]
fn never_overflows_random_lines() {
    let mut rng = Rng(42);
    for cap in [16usize, 64, 127, 128, 255] {
        let mut s = Streamer::new(cap);
        let mut model: std::collections::VecDeque<usize> = Default::default();
        for _ in 0..50_000 {
            if rng.below(3) == 0 {
                let got = s.ack();
                assert_eq!(got, model.pop_front());
            } else {
                let len = rng.below(90) as usize;
                if s.push(len) {
                    model.push_back(len + 1);
                }
            }
            assert!(s.used() <= cap);
            assert_eq!(s.used(), model.iter().sum::<usize>());
        }
    }
}
