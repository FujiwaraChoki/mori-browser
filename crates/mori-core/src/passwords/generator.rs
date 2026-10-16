//! Strong passwords for sign-up and change-password forms.

use super::Secret;

/// Letters and digits that can't be mistaken for each other (no 0/O/o,
/// 1/l/I).
const ALPHABET: &[u8] = b"abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const GROUP: usize = 6;
const GROUPS: usize = 3;
/// Length of the grouped form, `xxxxxx-xxxxxx-xxxxxx`.
const GROUPED_LEN: usize = GROUP * GROUPS + GROUPS - 1;
/// The shortest password worth suggesting.
pub const MIN_LEN: usize = 8;

/// A random password with lower- and uppercase letters and a digit, from
/// the operating system's secure random source: `abcDef-ghi2kl-mnopqr` style
/// (about 104 bits), or plain letters and digits when the field only takes
/// `max_len` characters. `None` if the system has no random source, or
/// the field takes fewer than [`MIN_LEN`] characters (no strong password
/// fits, and a longer one wouldn't be what the site stores).
pub fn generate_password(max_len: Option<usize>) -> Option<Secret> {
    let len = match max_len {
        Some(max) if max < MIN_LEN => return None,
        Some(max) if max < GROUPED_LEN => max,
        _ => GROUPED_LEN,
    };
    let grouped = len == GROUPED_LEN;
    let chars = if grouped { GROUP * GROUPS } else { len };
    loop {
        let mut picked = Vec::with_capacity(chars);
        while picked.len() < chars {
            let mut bytes = [0u8; 64];
            getrandom::fill(&mut bytes).ok()?;
            // Rejection sampling keeps every character equally likely.
            let limit = 256 - 256 % ALPHABET.len();
            picked.extend(
                bytes
                    .iter()
                    .filter(|b| (**b as usize) < limit)
                    .map(|b| ALPHABET[*b as usize % ALPHABET.len()])
                    .take(chars - picked.len()),
            );
            zeroize::Zeroize::zeroize(&mut bytes);
        }
        let ok = picked.iter().any(u8::is_ascii_lowercase)
            && picked.iter().any(u8::is_ascii_uppercase)
            && picked.iter().any(u8::is_ascii_digit);
        let mut out = String::with_capacity(len);
        if ok {
            for (i, c) in picked.iter().enumerate() {
                if grouped && i > 0 && i % GROUP == 0 {
                    out.push('-');
                }
                out.push(*c as char);
            }
        }
        zeroize::Zeroize::zeroize(&mut picked);
        if ok {
            return Some(Secret::new(out));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouped_passwords() {
        for _ in 0..200 {
            let p = generate_password(None).unwrap();
            let p = p.expose();
            assert_eq!(p.len(), GROUPED_LEN);
            let groups: Vec<&str> = p.split('-').collect();
            assert_eq!(groups.len(), GROUPS);
            assert!(groups.iter().all(|g| g.len() == GROUP && g.bytes().all(|b| ALPHABET.contains(&b))));
            assert!(p.bytes().any(|b| b.is_ascii_uppercase()));
            assert!(p.bytes().any(|b| b.is_ascii_lowercase()));
            assert!(p.bytes().any(|b| b.is_ascii_digit()));
        }
    }

    #[test]
    fn short_fields_get_plain_passwords() {
        let p = generate_password(Some(12)).unwrap();
        assert_eq!(p.expose().len(), 12);
        assert!(!p.expose().contains('-'));
        assert_eq!(generate_password(Some(8)).unwrap().expose().len(), 8);
        assert_eq!(generate_password(Some(7)), None, "too short to be strong");
        assert_eq!(generate_password(Some(2)), None);
        assert_eq!(generate_password(Some(200)).unwrap().expose().len(), GROUPED_LEN);
    }

    #[test]
    fn passwords_differ() {
        let a = generate_password(None).unwrap();
        let b = generate_password(None).unwrap();
        assert_ne!(a, b);
    }
}
