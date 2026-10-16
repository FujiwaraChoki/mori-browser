//! Importing and exporting logins as CSV.
//!
//! Reads the exports of Chrome (and Chromium browsers, Arc included:
//! `name,url,username,password,note`), Firefox (`url,username,password,
//! httpRealm,…`), Safari and Bitwarden (`login_uri,login_username,
//! login_password`): columns are found by their header. Writes Chrome's
//! format, which every browser imports.

use zeroize::Zeroizing;

use super::{Secret, login_origin};

/// One login read from a CSV file.
#[derive(Debug, PartialEq)]
pub struct ImportedLogin {
    pub origin: String,
    pub username: String,
    pub password: Secret,
    /// When the source browser saved it (UNIX seconds), if it says.
    pub created: Option<f64>,
}

/// The logins in a CSV file, and how many rows were left out (no web
/// address, no password, HTTP authentication entries).
#[derive(Debug, Default, PartialEq)]
pub struct CsvImport {
    pub logins: Vec<ImportedLogin>,
    pub skipped: usize,
}

type Row = Zeroizing<Vec<String>>;

/// The trimmed cell `i` of `row`, empty when the row is short.
fn cell(row: &Row, i: usize) -> &str {
    row.get(i).map(|s| s.trim()).unwrap_or("")
}

/// Split CSV text (RFC 4180: quoted fields, doubled quotes, line breaks in
/// quotes, CRLF or LF) into rows. Blank lines are dropped.
fn parse_rows(text: &str) -> Vec<Row> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = Vec::new();
    let mut row: Row = Zeroizing::new(Vec::new());
    let mut field = Zeroizing::new(String::new());
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                c => field.push(c),
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => row.push(std::mem::take(&mut *field)),
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                row.push(std::mem::take(&mut *field));
                if !(row.len() == 1 && row[0].is_empty()) {
                    rows.push(std::mem::replace(&mut row, Zeroizing::new(Vec::new())));
                } else {
                    row.clear();
                }
            }
            c => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(std::mem::take(&mut *field));
        rows.push(row);
    }
    rows
}

/// Read a password export. Fails when the file has no recognizable header
/// (a web address, username and password column).
pub fn import_csv(text: &str) -> Result<CsvImport, String> {
    let mut rows = parse_rows(text).into_iter();
    let header: Vec<String> = rows
        .next()
        .ok_or_else(|| "The file is empty.".to_string())?
        .iter()
        .map(|h| h.trim().to_ascii_lowercase())
        .collect();
    let column = |names: &[&str]| header.iter().position(|h| names.contains(&h.as_str()));
    let (Some(url_col), Some(user_col), Some(pass_col)) = (
        column(&["url", "login_uri", "website", "web site", "uri", "origin"]),
        column(&["username", "login_username", "user name", "user", "login"]),
        column(&["password", "login_password"]),
    ) else {
        return Err("This doesn't look like a password export: it needs url, username and password columns.".into());
    };
    let realm_col = column(&["httprealm"]);
    let created_col = column(&["timecreated"]);

    let mut import = CsvImport::default();
    for mut row in rows {
        let origin = login_origin(cell(&row, url_col));
        let realm = realm_col.map(|i| cell(&row, i)).unwrap_or("");
        let (Some(origin), true, true) = (origin, realm.is_empty(), !cell(&row, pass_col).is_empty()) else {
            import.skipped += 1;
            continue;
        };
        let username = cell(&row, user_col).to_string();
        // Firefox: milliseconds since the epoch.
        let created = created_col
            .and_then(|i| cell(&row, i).parse::<f64>().ok())
            .filter(|ms| ms.is_finite() && *ms > 0.)
            .map(|ms| ms / 1000.);
        let password = Secret::new(std::mem::take(&mut row[pass_col]));
        // A later row for the same login wins, as in the source browser.
        import.logins.retain(|l| !(l.origin == origin && l.username == username));
        import.logins.push(ImportedLogin { origin, username, password, created });
    }
    Ok(import)
}

fn push_field(out: &mut String, value: &str) {
    if value.contains([',', '"', '\n', '\r']) || value.trim() != value {
        out.push('"');
        out.push_str(&value.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(value);
    }
}

/// A Chrome-format export of `(origin, username, password)` logins.
pub fn export_csv<'a>(logins: impl IntoIterator<Item = (&'a str, &'a str, &'a Secret)>) -> Secret {
    let mut out = String::from("name,url,username,password,note\n");
    for (origin, username, password) in logins {
        let name = super::display_origin(origin);
        for (i, value) in [name.as_str(), origin, username, password.expose()].into_iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            push_field(&mut out, value);
        }
        out.push_str(",\n");
    }
    Secret::new(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(import: &CsvImport) -> Vec<(String, String, String)> {
        import
            .logins
            .iter()
            .map(|l| (l.origin.clone(), l.username.clone(), l.password.expose().to_string()))
            .collect()
    }

    #[test]
    fn rows_follow_rfc4180() {
        let rows = parse_rows("\u{feff}a,\"b,c\",\"say \"\"hi\"\"\"\r\n\r\n1,\"two\nlines\",\n");
        let rows: Vec<Vec<String>> = rows.into_iter().map(|r| r.to_vec()).collect();
        assert_eq!(rows, vec![vec!["a", "b,c", "say \"hi\""], vec!["1", "two\nlines", ""]]);
        assert_eq!(parse_rows("x,y").len(), 1, "no trailing newline");
        assert!(parse_rows("").is_empty());
    }

    #[test]
    fn chrome_export() {
        let csv = "name,url,username,password,note\n\
            github.com,https://github.com/login,octocat,\"pa,ss\"\"word\",\n\
            app,android://abc@com.example/,me,secret,\n\
            example.com,https://example.com/,,only-password,\n\
            empty,https://empty.test/,me,,\n";
        let import = import_csv(csv).unwrap();
        assert_eq!(
            summary(&import),
            [
                ("https://github.com".into(), "octocat".into(), "pa,ss\"word".into()),
                ("https://example.com".into(), "".into(), "only-password".into()),
            ]
        );
        assert_eq!(import.skipped, 2);
    }

    #[test]
    fn firefox_export() {
        let csv = "\"url\",\"username\",\"password\",\"httpRealm\",\"formActionOrigin\",\"guid\",\"timeCreated\",\"timeLastUsed\",\"timePasswordChanged\"\r\n\
            \"https://accounts.example.com\",\"me@example.com\",\"hunter2\",,\"https://accounts.example.com\",\"{1}\",\"1700000000000\",\"1700000000000\",\"1700000000000\"\r\n\
            \"https://intranet.test\",\"admin\",\"basic\",\"Intranet\",,\"{2}\",\"1\",\"1\",\"1\"\r\n";
        let import = import_csv(csv).unwrap();
        assert_eq!(summary(&import), [("https://accounts.example.com".into(), "me@example.com".into(), "hunter2".into())]);
        assert_eq!(import.logins[0].created, Some(1_700_000_000.));
        assert_eq!(import.skipped, 1, "HTTP auth entries are left out");
    }

    #[test]
    fn bitwarden_columns_and_duplicates() {
        let csv = "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n\
            ,,login,A,,,0,https://a.test/x,me,one,\n\
            ,,login,A,,,0,https://a.test/y,me,two,\n";
        let import = import_csv(csv).unwrap();
        assert_eq!(summary(&import), [("https://a.test".into(), "me".into(), "two".into())]);
    }

    #[test]
    fn rejects_other_files() {
        assert!(import_csv("").is_err());
        assert!(import_csv("title,artist\nSong,Band\n").is_err());
    }

    #[test]
    fn export_roundtrips() {
        let p1 = Secret::new("pa,ss\"word".into());
        let p2 = Secret::new(" spaced ".into());
        let csv = export_csv([("https://a.test:8443", "me", &p1), ("https://b.test", "", &p2)]);
        assert!(csv.expose().starts_with("name,url,username,password,note\na.test:8443,https://a.test:8443,me,\"pa,ss\"\"word\",\n"));
        let back = import_csv(csv.expose()).unwrap();
        assert_eq!(
            summary(&back),
            [
                ("https://a.test:8443".into(), "me".into(), "pa,ss\"word".into()),
                ("https://b.test".into(), "".into(), " spaced ".into()),
            ]
        );
    }
}
