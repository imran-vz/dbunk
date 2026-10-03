use super::*;

#[test]
fn refused_choice_capture_never_enters_the_cloning_pass() {
    let visits = Rc::new(Cell::new(0));
    let counted = visits.clone();
    let rows =
        std::iter::once(("owned", " Exact 資料 ")).inspect(move |_| counted.set(counted.get() + 1));
    let budget = Rc::new(Cell::new(128 * MIB));
    assert!(Connections::capture(rows.clone(), budget.clone()).is_err());
    assert_eq!(visits.get(), 2); // Borrowed count and validation only.
    assert_eq!(budget.get(), 128 * MIB);
    budget.set(127 * MIB);
    visits.set(0);
    let choices = Connections::capture(rows, budget.clone()).unwrap();
    assert_eq!(visits.get(), 3);
    assert_eq!(choices.rows[0].1, " Exact 資料 ");
    assert_eq!(budget.get(), 128 * MIB);
    drop(choices);
    assert_eq!(budget.get(), 127 * MIB);
}

#[test]
fn replacement_admission_charges_old_and_new_and_preserves_old_on_refusal() {
    let budget = Rc::new(Cell::new(126 * MIB));
    let fields = Lease::new(budget.clone(), MIB).unwrap();
    let previous = Lease::new(budget.clone(), MIB).unwrap();
    assert!(Lease::new(budget.clone(), MIB).is_none());
    assert_eq!(budget.get(), 128 * MIB);
    drop(fields);
    let replacement = Lease::new(budget.clone(), MIB).unwrap();
    assert_eq!(budget.get(), 128 * MIB);
    drop(previous);
    assert_eq!(budget.get(), 127 * MIB);
    drop(replacement);
    assert_eq!(budget.get(), 126 * MIB);
}

#[test]
fn connection_choices_reject_duplicates_and_hidden_capacity_without_normalizing_names() {
    let choices = vec![("exact-id".into(), " Name 資料 ".into())];
    assert!(valid_connections(&choices));
    assert_eq!(choices[0].1, " Name 資料 ");
    assert!(!valid_connections(&vec![
        choices[0].clone(),
        choices[0].clone()
    ]));
    let mut inflated = choices.clone();
    inflated[0].1.reserve(MIB);
    assert!(!valid_connections(&inflated));
    let mut excessive = Vec::with_capacity(1025);
    excessive.push(choices[0].clone());
    assert!(!valid_connections(&excessive));
    assert!(valid_name(&"界".repeat(21)));
    assert!(!valid_name(&"界".repeat(22)));
    assert!(valid_name(" Exact \" identifier "));
    assert!(!valid_name("\0"));
    assert!(valid_name("  "));
}
