//! Sorting recordings into groups (by channel or by programme title) with a heading row before each group.
use tvhme_core::api::Recording;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GroupBy { None, Channel, Series }

impl GroupBy {
    pub fn from_index(i: i32) -> Self { match i { 1 => Self::Channel, 2 => Self::Series, _ => Self::None } }
}

/// One line of the list: a group heading (with how many recordings it holds) or a recording.
#[derive(Clone, Debug, PartialEq)]
pub enum Line<'a> { Heading(String, usize), Item(&'a Recording) }

fn key(r: &Recording, by: GroupBy) -> String {
    let k = match by { GroupBy::Channel => &r.channel_name, _ => &r.title };
    let k = k.trim();
    if k.is_empty() { "Unknown".to_string() } else { k.to_string() }
}

/// Without grouping the list keeps the server's order. With grouping, groups are alphabetical (ignoring case)
/// and the recordings inside each one run oldest to newest.
pub fn arrange(list: &[Recording], by: GroupBy) -> Vec<Line<'_>> {
    if by == GroupBy::None { return list.iter().map(Line::Item).collect(); }
    let mut sorted: Vec<&Recording> = list.iter().collect();
    sorted.sort_by(|a, b| key(a, by).to_lowercase().cmp(&key(b, by).to_lowercase()).then(a.start.cmp(&b.start)));
    let mut out = Vec::new();
    let mut i = 0;
    while i < sorted.len() {
        let k = key(sorted[i], by);
        let n = sorted[i..].iter().take_while(|r| key(r, by).eq_ignore_ascii_case(&k)).count();
        out.push(Line::Heading(k, n));
        out.extend(sorted[i..i + n].iter().map(|r| Line::Item(r)));
        i += n;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn r(ch: &str, title: &str, start: i64) -> Recording {
        Recording { uuid: format!("{ch}{title}{start}"), channel_name: ch.into(), title: title.into(), subtitle: String::new(), description: String::new(), start, stop: start + 60, status: String::new(), sched_status: String::new(), filesize: 0, errors: 0, enabled: true }
    }
    #[test]
    fn no_grouping_keeps_order() {
        let l = vec![r("B", "x", 5), r("A", "y", 1)];
        assert_eq!(arrange(&l, GroupBy::None).len(), 2);
        assert!(matches!(arrange(&l, GroupBy::None)[0], Line::Item(x) if x.channel_name == "B"));
    }
    #[test]
    fn by_channel_groups_sorted_with_counts() {
        let l = vec![r("beta", "n", 9), r("Alpha", "n", 5), r("alpha", "m", 2), r("", "q", 1)];
        let a = arrange(&l, GroupBy::Channel);
        assert_eq!(a[0], Line::Heading("alpha".into(), 2)); // spelled like its oldest recording
        assert!(matches!(a[1], Line::Item(x) if x.start == 2));   // oldest first
        assert!(matches!(a[2], Line::Item(x) if x.start == 5));
        assert_eq!(a[3], Line::Heading("beta".into(), 1));
        assert_eq!(a[5], Line::Heading("Unknown".into(), 1)); 
    }
    #[test]
    fn by_series_uses_the_title() {
        let l = vec![r("A", "News", 3), r("B", "News", 1), r("A", "Film", 2)];
        let a = arrange(&l, GroupBy::Series);
        assert_eq!(a[0], Line::Heading("Film".into(), 1));
        assert_eq!(a[2], Line::Heading("News".into(), 2));
        assert!(matches!(a[3], Line::Item(x) if x.channel_name == "B"));
    }
}
