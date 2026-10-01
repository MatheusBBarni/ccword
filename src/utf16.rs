/// UTF-16 length of `text`, matching `NSString.length`.
pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// Convert a char-index range into a UTF-16 range.
///
/// `NSSpellChecker` ranges are UTF-16 code units, not bytes, chars, or
/// grapheme clusters. The range must not split a surrogate pair; char
/// boundaries never do, because a non-BMP scalar is one `char` and two
/// UTF-16 units.
pub fn char_range_to_utf16(
    text: &str,
    start_chars: usize,
    end_chars: usize,
) -> Option<(usize, usize)> {
    if start_chars > end_chars {
        return None;
    }
    let mut chars = 0;
    let mut units = 0;
    let mut start_units = None;
    for ch in text.chars() {
        if chars == start_chars {
            start_units = Some(units);
        }
        if chars == end_chars {
            let start = start_units?;
            return Some((start, units - start));
        }
        units += ch.encode_utf16(&mut [0; 2]).len();
        chars += 1;
    }
    if chars == end_chars {
        let start = if start_chars == end_chars {
            Some(units)
        } else {
            start_units
        }?;
        return Some((start, units - start));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_range_matches_char_indexes() {
        assert_eq!(char_range_to_utf16("help", 0, 2), Some((0, 2)));
    }

    #[test]
    fn emoji_counts_as_one_char_and_two_utf16_units() {
        let text = "a😀b";
        assert_eq!(utf16_len(text), 4);
        assert_eq!(char_range_to_utf16(text, 1, 2), Some((1, 2)));
        assert_eq!(char_range_to_utf16(text, 2, 3), Some((3, 1)));
    }

    #[test]
    fn combining_mark_is_its_own_char_and_does_not_split_a_surrogate() {
        let text = "e\u{0301}😀";
        assert_eq!(text.chars().count(), 3);
        let (loc, len) = char_range_to_utf16(text, 2, 3).unwrap();
        assert_eq!((loc, len), (2, 2));
        let units: Vec<u16> = text.encode_utf16().collect();
        assert!(units[loc] >= 0xD800 && units[loc] <= 0xDBFF);
    }

    #[test]
    fn rejects_a_range_past_the_end() {
        assert_eq!(char_range_to_utf16("ab", 0, 3), None);
        assert_eq!(char_range_to_utf16("ab", 2, 1), None);
    }
}
