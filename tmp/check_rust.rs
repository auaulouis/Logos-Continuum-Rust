fn main() {
    let body_text = "accounted-for losses conservatively estimated at 35%.";
    let paragraph = format!("{} {}", "", body_text).to_ascii_lowercase();
    let group = vec!["republicans", "conservative", "conservatives", "republican", "gop"];
    let any_matched = group.iter().any(|term| paragraph.contains(*term));
    println!("any_matched: {}", any_matched);
}
