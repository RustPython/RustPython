use crate::source_code::SourceLocation;

// pub(crate) fn new_location_error(
//     index: usize,
//     field: &str,
//     vm: &VirtualMachine,
// ) -> PyRef<PyBaseException> {
//     vm.new_value_error(format!("value {index} is too large for location {field}"))
// }

pub(crate) struct AtLocation<'a>(pub Option<&'a SourceLocation>);

impl std::fmt::Display for AtLocation<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (row, column) = self
            .0
            .map_or((0, 0), |l| (l.row.to_usize(), l.column.to_usize()));
        write!(f, " at line {row} column {column}",)
    }
}
