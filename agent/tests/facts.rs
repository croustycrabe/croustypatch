//! Tests for `facts`.
use croustypatch_agent::facts::*;

#[test]
fn os_release_parsing() {
    let m = parse_os_release("NAME=\"Arch Linux\"\n# c\nID=arch\nBUILD_ID=rolling\nVERSION_ID='12'\n\nbroken line\n");
    assert_eq!(m["NAME"], "Arch Linux");
    assert_eq!(m["BUILD_ID"], "rolling");
    assert_eq!(m["VERSION_ID"], "12");
    assert!(!m.contains_key("broken line"));
}
