fn checked_eq_failure<L: ?Sized, R: ?Sized>(_left: &L, _right: &R) -> !
    requires false,
{
    panic!("assertion failed");
}
