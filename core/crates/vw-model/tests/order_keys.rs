//! Order keys: long append and prepend runs stay short, every generated key is
//! strictly inside its bounds, and keys made by the first (bisecting) version
//! of the generator keep their meaning.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use vw_model::OrderKey;

const DIGITS: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

fn key(text: &str) -> OrderKey {
    OrderKey::try_from(text.to_owned()).unwrap()
}

/// The generator this crate shipped first: bisect the first digit with room.
/// Kept here as the reference for which keys must not change.
fn original(left: Option<&str>, right: Option<&str>) -> Option<String> {
    let value = |byte: u8| DIGITS.iter().position(|d| *d == byte).unwrap() as u8;
    let lower = left.map_or(&[][..], str::as_bytes);
    let upper = right.map(str::as_bytes);
    let mut upper_open = upper.is_none();
    let mut out = String::new();
    for index in 0..128 {
        let lo = lower.get(index).map_or(0, |b| value(*b));
        let hi = if upper_open {
            62
        } else {
            upper.and_then(|v| v.get(index)).map_or(0, |b| value(*b))
        };
        if hi > lo + 1 {
            out.push(char::from(DIGITS[usize::from(lo + (hi - lo) / 2)]));
            return Some(out);
        }
        out.push(char::from(DIGITS[usize::from(lo)]));
        if lo < hi {
            upper_open = true;
        }
    }
    None
}

struct Random(u64);
impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
    /// A valid key that often has long runs of the end digits `0` and `z`,
    /// which is where the open-ended generators change behaviour.
    fn key(&mut self, longest: u64) -> OrderKey {
        let length = 1 + self.below(longest) as usize;
        let style = self.below(4);
        let mut text = String::with_capacity(length);
        for index in 0..length {
            let digit = match style {
                0 => self.below(62),
                1 if index < length / 2 => 61,
                2 if index < length / 2 => 0,
                3 => [0, 0, 61, 61, 1, 60, self.below(62)][self.below(7) as usize],
                _ => self.below(62),
            } as usize;
            text.push(char::from(DIGITS[digit]));
        }
        if text.ends_with('0') {
            text.pop();
            text.push(char::from(DIGITS[1 + self.below(61) as usize]));
        }
        key(&text)
    }
}

#[test]
fn the_first_keys_are_the_ones_the_original_generator_made() {
    let mut appended = vec![OrderKey::between(None, None).unwrap()];
    for _ in 0..6 {
        appended.push(OrderKey::between(appended.last(), None).unwrap());
    }
    assert_eq!(
        appended.iter().map(OrderKey::as_str).collect::<Vec<_>>(),
        ["V", "k", "s", "w", "y", "z", "z0001"]
    );
    let mut prepended = vec![key("V")];
    for _ in 0..6 {
        prepended.push(OrderKey::between(None, prepended.last()).unwrap());
    }
    assert_eq!(
        prepended.iter().map(OrderKey::as_str).collect::<Vec<_>>(),
        ["V", "F", "7", "3", "1", "0V", "0Uzzz"]
    );
    // Everything the original made for these inputs is still what is made now.
    let mut left: Option<String> = None;
    for _ in 0..6 {
        let expected = original(left.as_deref(), None).unwrap();
        let actual = OrderKey::between(left.as_deref().map(key).as_ref(), None).unwrap();
        assert_eq!(actual.as_str(), expected);
        left = Some(expected);
    }
    let mut right = "V".to_owned();
    for _ in 0..5 {
        let expected = original(None, Some(&right)).unwrap();
        let actual = OrderKey::between(None, Some(&key(&right))).unwrap();
        assert_eq!(actual.as_str(), expected);
        right = expected;
    }
}

#[test]
fn the_original_generator_ran_out_after_768_appends() {
    // The defect this replaces: six keys per digit, 128 digits, then failure.
    let mut last: Option<String> = None;
    let mut made = 0;
    while let Some(next) = original(last.as_deref(), None) {
        made += 1;
        last = Some(next);
    }
    assert_eq!(made, 768);
    assert_eq!(last.unwrap(), "z".repeat(128));
}

#[test]
fn one_hundred_thousand_appends_keep_short_strictly_increasing_keys() {
    let mut last = OrderKey::between(None, None).unwrap();
    let mut longest = last.as_str().len();
    for _ in 0..100_000 {
        let next = OrderKey::between(Some(&last), None).unwrap();
        assert!(last < next, "{} !< {}", last.as_str(), next.as_str());
        assert_eq!(key(next.as_str()), next);
        longest = longest.max(next.as_str().len());
        last = next;
    }
    assert!(longest <= 5, "longest appended key has {longest} digits");
}

#[test]
fn one_hundred_thousand_prepends_keep_short_strictly_decreasing_keys() {
    let mut first = OrderKey::between(None, None).unwrap();
    let mut longest = first.as_str().len();
    for _ in 0..100_000 {
        let next = OrderKey::between(None, Some(&first)).unwrap();
        assert!(next < first, "{} !< {}", next.as_str(), first.as_str());
        assert_eq!(key(next.as_str()), next);
        longest = longest.max(next.as_str().len());
        first = next;
    }
    assert!(longest <= 5, "longest prepended key has {longest} digits");
}

#[test]
fn several_million_appends_and_prepends_stay_under_ten_digits() {
    // Step through the counter in large strides rather than one key at a time:
    // each run of end digits holds 61 * 62^3 keys, so crossing three run
    // lengths proves the carry into a longer run without 40 million calls.
    let mut last = key("z");
    let mut first = key("1");
    for _ in 0..3 {
        // The last key of this run length: appending carries into the next.
        let run = last.as_str().bytes().take_while(|b| *b == b'z').count();
        last = key(&format!("{}yzzz", "z".repeat(run)));
        let next = OrderKey::between(Some(&last), None).unwrap();
        assert_eq!(next.as_str(), "z".repeat(run + 1));
        let after = OrderKey::between(Some(&next), None).unwrap();
        assert_eq!(after.as_str(), format!("{}0001", "z".repeat(run + 1)));
        last = after;

        let run = first.as_str().bytes().take_while(|b| *b == b'0').count();
        first = key(&format!("{}1", "0".repeat(run)));
        let next = OrderKey::between(None, Some(&first)).unwrap();
        let expected = if run == 0 {
            "0V".to_owned()
        } else {
            format!("{}zzz", "0".repeat(run + 1))
        };
        assert_eq!(next.as_str(), expected);
        first = next;
    }
    assert!(last.as_str().len() < 10 && first.as_str().len() < 10);
}

#[test]
fn interleaved_appends_prepends_and_inserts_keep_strict_order() {
    for seed in 0..8_u64 {
        let mut random = Random(seed + 1);
        let mut keys = vec![OrderKey::between(None, None).unwrap()];
        for step in 0..12_000_u32 {
            // Mostly the ends, as drawing and stacking do, with inserts between.
            let index = match random.below(10) {
                0..=4 => keys.len(),
                5..=6 => 0,
                7 => keys.len() - 1,
                8 => 1.min(keys.len()),
                _ => random.below(keys.len() as u64 + 1) as usize,
            };
            let left = index.checked_sub(1).and_then(|i| keys.get(i));
            let right = keys.get(index);
            let middle = OrderKey::between(left, right);
            assert!(middle.is_ok(), "seed {seed} step {step} has no key");
            let middle = middle.unwrap();
            assert!(left.is_none_or(|v| v < &middle));
            assert!(right.is_none_or(|v| &middle < v));
            keys.insert(index, middle);
        }
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
        let longest = keys.iter().map(|k| k.as_str().len()).max().unwrap();
        assert!(
            longest <= 16,
            "seed {seed}: longest key has {longest} digits"
        );
    }
}

#[test]
fn a_key_between_random_valid_keys_is_strictly_between_or_cannot_exist() {
    let mut random = Random(0x5eed);
    for round in 0..200_000_u32 {
        let longest = if round % 4 == 0 { 128 } else { 12 };
        let a = random.key(longest);
        let b = random.key(longest);
        if a == b {
            assert!(OrderKey::between(Some(&a), Some(&b)).is_err());
            continue;
        }
        let (low, high) = if a < b { (a, b) } else { (b, a) };
        assert!(OrderKey::between(Some(&high), Some(&low)).is_err());
        match OrderKey::between(Some(&low), Some(&high)) {
            Ok(middle) => {
                assert!(low < middle && middle < high);
                assert_eq!(key(middle.as_str()), middle);
                // Two bounds are bisected exactly as before.
                assert_eq!(
                    Some(middle.as_str()),
                    original(Some(low.as_str()), Some(high.as_str())).as_deref()
                );
            }
            Err(_) => {
                // Only full-length neighbours have nothing between them.
                assert_eq!(high.as_str().len().max(low.as_str().len()), 128);
                assert!(original(Some(low.as_str()), Some(high.as_str())).is_none());
            }
        }
        for bound in [&low, &high] {
            match OrderKey::between(Some(bound), None) {
                Ok(above) => {
                    assert!(bound < &above);
                    assert_eq!(key(above.as_str()), above);
                    assert!(above.as_str().len() <= bound.as_str().len() + 4);
                }
                Err(_) => assert_eq!(bound.as_str(), "z".repeat(128)),
            }
            match OrderKey::between(None, Some(bound)) {
                Ok(below) => {
                    assert!(&below < bound);
                    assert_eq!(key(below.as_str()), below);
                    assert!(below.as_str().len() <= bound.as_str().len() + 4);
                }
                Err(_) => assert_eq!(bound.as_str(), format!("{}1", "0".repeat(127))),
            }
        }
    }
}

#[test]
fn full_length_neighbours_have_no_key_between_them() {
    let prefix = "V".repeat(127);
    let low = key(&format!("{prefix}y"));
    let high = key(&format!("{prefix}z"));
    assert!(OrderKey::between(Some(&low), Some(&high)).is_err());
    let low = key("V");
    let high = key(&format!("V{}1", "0".repeat(126)));
    assert!(OrderKey::between(Some(&low), Some(&high)).is_err());
    // One digit shorter and there is room again.
    let high = key(&format!("V{}1", "0".repeat(125)));
    let middle = OrderKey::between(Some(&low), Some(&high)).unwrap();
    assert!(low < middle && middle < high);
}

#[test]
fn documents_filled_by_the_original_generator_keep_taking_appends() {
    // Every key the original generator could have left as a layer's top key.
    let mut legacy = Vec::new();
    let mut last: Option<String> = None;
    while let Some(next) = original(last.as_deref(), None) {
        legacy.push(next.clone());
        last = Some(next);
    }
    assert_eq!(legacy.len(), 768);
    let mut nearly_full = Vec::new();
    for (count, top) in legacy.iter().enumerate() {
        let mut current = key(top);
        let mut appended = 0;
        while appended < 300 {
            match OrderKey::between(Some(&current), None) {
                Ok(next) => {
                    assert!(current < next);
                    assert!(next.as_str().len() <= (top.len() + 4).min(128));
                    current = next;
                    appended += 1;
                }
                Err(_) => {
                    assert_eq!(current.as_str(), "z".repeat(128));
                    break;
                }
            }
        }
        if appended < 300 {
            nearly_full.push((count + 1, appended));
        }
    }
    // Only a layer the original generator had filled to within nine objects
    // of its 768 limit has fewer than 300 appends left; at the limit itself
    // (the 128-digit all-z key) no larger key exists at all.
    assert_eq!(
        nearly_full,
        [
            (760, 247),
            (761, 123),
            (762, 61),
            (763, 30),
            (764, 15),
            (765, 7),
            (766, 3),
            (767, 1),
            (768, 0)
        ]
    );
}

#[test]
fn appending_after_a_128_digit_key_works_unless_it_is_the_largest_key() {
    // A full-length key that is not all z: the next key is short again.
    let long = key(&"V".repeat(128));
    assert_eq!(OrderKey::between(Some(&long), None).unwrap().as_str(), "k");
    assert_eq!(OrderKey::between(None, Some(&long)).unwrap().as_str(), "F");
    // A full-length key deep inside a run of z: later keys share the run and
    // count in the four digits after it.
    let deep = key(&format!("{}{}", "z".repeat(100), "V".repeat(28)));
    let next = OrderKey::between(Some(&deep), None).unwrap();
    assert_eq!(next.as_str(), format!("{}VVVW", "z".repeat(100)));
    // One digit of room left: it counts up to the largest key, then stops.
    let mut current = key(&format!("{}V", "z".repeat(127)));
    let mut appended = 0;
    while let Ok(next) = OrderKey::between(Some(&current), None) {
        assert!(current < next);
        current = next;
        appended += 1;
    }
    assert_eq!((appended, current.as_str()), (30, "z".repeat(128).as_str()));
    // Mirror image at the low end.
    let mut current = key(&format!("{}V", "0".repeat(127)));
    let mut prepended = 0;
    while let Ok(next) = OrderKey::between(None, Some(&current)) {
        assert!(next < current);
        current = next;
        prepended += 1;
    }
    assert_eq!(
        (prepended, current.as_str()),
        (30, format!("{}1", "0".repeat(127)).as_str())
    );
}
