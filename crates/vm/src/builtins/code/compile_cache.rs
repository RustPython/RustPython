//! Compilation-private sharing of supported constants and surviving assembly fields.
//!
//! This sees the optimized Basic graph, not CPython's full first-winner chronology.
//! Code/NaN keys and whole slice subtrees deliberately remain unmerged. The pool is
//! never used by public readers, frozen decoding, constructors or replace().

use super::*;
use alloc::collections::BTreeMap;
use core::cell::RefCell;

// Keys contain no Python owners. Bit keys preserve each signed-zero combination;
// NaNs are excluded because their identity cannot be represented by their bits.
#[derive(Eq, PartialEq, Ord, PartialOrd)]
enum Key {
    None,
    Ellipsis,
    Bool(bool),
    Int(BigInt),
    Float(u64),
    Complex(u64, u64),
    Str(Vec<u8>),
    Bytes(Vec<u8>),
    Tuple(Vec<Self>),
    FrozenSet(Vec<Self>),
}

fn key(obj: &PyObject) -> Option<Key> {
    use crate::builtins::{PyBool, PyBytes, PyComplex, PyEllipsis, PyFloat, PyInt, PyStr, PyTuple};
    if obj
        .class()
        .is(super::super::singletons::PyNone::static_type())
    {
        Some(Key::None)
    } else if obj.class().is(PyEllipsis::static_type()) {
        Some(Key::Ellipsis)
    } else if obj.class().is(PyBool::static_type()) {
        Some(Key::Bool(
            !obj.downcast_ref::<PyInt>()?.as_bigint().is_zero(),
        ))
    } else if obj.class().is(PyInt::static_type()) {
        Some(Key::Int(obj.downcast_ref::<PyInt>()?.as_bigint().clone()))
    } else if obj.class().is(PyFloat::static_type()) {
        let value = obj.downcast_ref::<PyFloat>()?.to_f64();
        (!value.is_nan()).then(|| Key::Float(value.to_bits()))
    } else if obj.class().is(PyComplex::static_type()) {
        let value = obj.downcast_ref::<PyComplex>()?.as_complex();
        (!value.re.is_nan() && !value.im.is_nan())
            .then(|| Key::Complex(value.re.to_bits(), value.im.to_bits()))
    } else if obj.class().is(PyStr::static_type()) {
        Some(Key::Str(
            obj.downcast_ref::<PyStr>()?.as_wtf8().as_bytes().to_vec(),
        ))
    } else if obj.class().is(PyBytes::static_type()) {
        Some(Key::Bytes(
            obj.downcast_ref::<PyBytes>()?.as_bytes().to_vec(),
        ))
    } else if obj.class().is(PyTuple::static_type()) {
        obj.downcast_ref::<PyTuple>()?
            .iter()
            .map(|child| key(child))
            .collect::<Option<Vec<_>>>()
            .map(Key::Tuple)
    } else if obj.class().is(PyFrozenSet::static_type()) {
        // Actual survivors matter: Python set equality can collapse unlike types.
        // This scoped snapshot owns children only while producing Rust-only keys.
        // Never use Literal::borrow_constant: its frozenset/slice adapters leak.
        let elements = obj.downcast_ref::<PyFrozenSet>()?.elements();
        let mut keys = elements
            .iter()
            .map(|child| key(child))
            .collect::<Option<Vec<_>>>()?;
        keys.sort_unstable();
        drop(elements);
        Some(Key::FrozenSet(keys))
    } else {
        None
    }
}

type Pool = RefCell<BTreeMap<Key, PyObjectRef>>;

#[derive(Clone, Copy)]
struct CompileBag<'a> {
    vm: &'a VirtualMachine,
    pool: &'a Pool,
}

impl CompileBag<'_> {
    fn share(&self, candidate: PyObjectRef) -> PyObjectRef {
        let Some(key) = key(&candidate) else {
            return candidate;
        };
        let winner = { self.pool.borrow().get(&key).cloned() };
        if let Some(winner) = winner {
            // No RefCell guard remains when the candidate is released.
            winner
        } else {
            self.pool.borrow_mut().insert(key, candidate.clone());
            candidate
        }
    }

    fn finalize(&self, code: CodeObject) -> PyCode {
        let mut result = PyCode::new(code, &self.vm.ctx);
        // Pinned assemble.c surviving-field order. Input bytecode is not the
        // separately cached exported co_code and must not be pooled here.
        result.exceptiontable_bytes = self
            .share(result.exceptiontable_bytes.into())
            .downcast()
            .unwrap();
        result.linetable_bytes = self
            .share(result.linetable_bytes.into())
            .downcast()
            .unwrap();
        let names = self
            .share(result.names_tuple.clone().into())
            .downcast()
            .unwrap();
        result.retain_normalized_names(names, None);
        let constants = result
            .code
            .constants
            .tuple_owner()
            .expect("finalized constants")
            .0
            .clone();
        let constants = self.share(constants.into()).downcast().unwrap();
        result.code.constants = bytecode::Constants::from_tuple(LiteralTupleOwner(constants));
        let locals = self
            .share(result.localsplus_names_tuple.clone().into())
            .downcast()
            .unwrap();
        // An equal-text pool winner is not necessarily normalized. Preserve the
        // safe copy fallback rather than changing published tuple elements.
        let names = result.names_tuple.clone();
        result.retain_normalized_names(names, Some(locals));
        result
    }

    fn wrap(&self, code: CodeObject) -> PyRef<PyCode> {
        PyRef::new_ref(
            self.finalize(code),
            self.vm.ctx.types.code_type.to_owned(),
            None,
        )
    }
}

impl ConstantBag for CompileBag<'_> {
    type Constant = Literal;

    fn make_constant<C: Constant>(&self, constant: BorrowedConstant<'_, C>) -> Literal {
        let obj = match constant {
            BorrowedConstant::Code { code } => {
                return Literal(self.wrap(code.map_clone_bag(self)).into());
            }
            // Delegate the entire subtree, including children, without caching.
            BorrowedConstant::Slice { .. } => return PyVmBag(self.vm).make_constant(constant),
            BorrowedConstant::Tuple { elements } => self
                .vm
                .ctx
                .new_tuple(
                    elements
                        .iter()
                        .map(|child| self.make_constant(child.borrow_constant()).0)
                        .collect(),
                )
                .into(),
            BorrowedConstant::Frozenset { elements } => PyFrozenSet::from_iter(
                self.vm,
                elements
                    .iter()
                    .map(|child| self.make_constant(child.borrow_constant()).0),
            )
            .unwrap()
            .into_ref(&self.vm.ctx)
            .into(),
            _ => PyVmBag(self.vm).make_constant(constant).0,
        };
        Literal(self.share(obj))
    }

    fn make_name(&self, name: &str) -> &'static PyStrInterned {
        self.vm.ctx.intern_str(name)
    }
    fn make_int(&self, value: BigInt) -> Literal {
        Literal(self.share(self.vm.ctx.new_int(value).into()))
    }
    fn make_tuple(&self, elements: impl Iterator<Item = Literal>) -> Literal {
        Literal(
            self.share(
                self.vm
                    .ctx
                    .new_tuple(elements.map(|value| value.0).collect())
                    .into(),
            ),
        )
    }
    fn make_code(&self, code: CodeObject) -> Literal {
        Literal(self.wrap(code).into())
    }
}

pub(super) fn into_pycode(code: bytecode::CodeObject, vm: &VirtualMachine) -> PyCode {
    let pool = Pool::default();
    let bag = CompileBag { vm, pool: &pool };
    let result = bag.finalize(code.map_bag(bag));
    // Nested PyRefs are still reachable only through this private result graph.
    // RAII also drops the pool on unwind; no cache owners escape publication.
    drop(pool);
    result
}

pub(super) fn into_code_object(code: bytecode::CodeObject, vm: &VirtualMachine) -> CodeObject {
    let pool = Pool::default();
    let result = code.map_bag(CompileBag { vm, pool: &pool });
    drop(pool);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytecode::ConstantData as Data;

    #[test]
    fn compilation_keys_preserve_types_zeros_and_exclusions() {
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let pool = Pool::default();
            let bag = CompileBag { vm, pool: &pool };
            let data = [
                Data::Boolean { value: false },
                Data::Integer { value: 0.into() },
                Data::Float { value: 0.0 },
                Data::Float { value: -0.0 },
                Data::Complex {
                    value: num_complex::Complex64::new(0.0, 0.0),
                },
                Data::Complex {
                    value: num_complex::Complex64::new(-0.0, 0.0),
                },
                Data::Complex {
                    value: num_complex::Complex64::new(0.0, -0.0),
                },
                Data::Complex {
                    value: num_complex::Complex64::new(-0.0, -0.0),
                },
            ];
            let values: Vec<_> = data
                .iter()
                .map(|value| bag.make_constant(value.borrow_constant()))
                .collect();
            for (index, value) in values.iter().enumerate() {
                let repeated = bag.make_constant(data[index].borrow_constant());
                assert!(value.0.is(&repeated.0));
                for other in &values[..index] {
                    assert!(!value.0.is(&other.0));
                }
            }
            let nan = Data::Tuple {
                elements: vec![Data::Float { value: f64::NAN }],
            };
            let first = bag.make_constant(nan.borrow_constant());
            let second = bag.make_constant(nan.borrow_constant());
            assert!(key(&first.0).is_none());
            assert!(!first.0.is(&second.0));
            for scalar in [
                Data::Complex {
                    value: num_complex::Complex64::new(f64::NAN, 1.0),
                },
                Data::Complex {
                    value: num_complex::Complex64::new(1.0, f64::NAN),
                },
            ] {
                let aggregate = Data::Tuple {
                    elements: vec![
                        scalar,
                        Data::Bytes {
                            value: b"safe sibling next to NaN".to_vec(),
                        },
                    ],
                };
                let first = bag.make_constant(aggregate.borrow_constant());
                let second = bag.make_constant(aggregate.borrow_constant());
                assert!(key(&first.0).is_none());
                assert!(!first.0.is(&second.0));
                let first = first.0.downcast_ref::<crate::builtins::PyTuple>().unwrap();
                let second = second.0.downcast_ref::<crate::builtins::PyTuple>().unwrap();
                assert!(!first.as_slice()[0].is(&second.as_slice()[0]));
                assert!(first.as_slice()[1].is(&second.as_slice()[1]));
            }
            let infinities = [
                Data::Float {
                    value: f64::INFINITY,
                },
                Data::Float {
                    value: f64::NEG_INFINITY,
                },
                Data::Complex {
                    value: num_complex::Complex64::new(f64::INFINITY, f64::NEG_INFINITY),
                },
                Data::Complex {
                    value: num_complex::Complex64::new(f64::NEG_INFINITY, f64::INFINITY),
                },
            ];
            let mut infinite_values: Vec<Literal> = Vec::new();
            for value in infinities {
                let first = bag.make_constant(value.borrow_constant());
                let second = bag.make_constant(value.borrow_constant());
                assert!(key(&first.0).is_some());
                assert!(first.0.is(&second.0));
                assert!(infinite_values.iter().all(|other| !first.0.is(&other.0)));
                infinite_values.push(first);
            }
            let bytes = || Data::Bytes {
                value: b"uncached slice child".to_vec(),
            };
            let slice = Data::Slice {
                elements: Box::new([bytes(), Data::None, Data::None]),
            };
            let first = bag.make_constant(slice.borrow_constant());
            let second = bag.make_constant(slice.borrow_constant());
            assert!(key(&first.0).is_none());
            let first = first.0.downcast_ref::<crate::builtins::PySlice>().unwrap();
            let second = second.0.downcast_ref::<crate::builtins::PySlice>().unwrap();
            assert!(
                !first
                    .start
                    .as_ref()
                    .unwrap()
                    .is(second.start.as_ref().unwrap())
            );
            let cached = bag.make_constant(bytes().borrow_constant());
            assert!(!first.start.as_ref().unwrap().is(&cached.0));
        });
    }

    #[test]
    fn compilation_frozenset_keys_use_survivors_and_release_pool_owners() {
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let pool = Pool::default();
            let bag = CompileBag { vm, pool: &pool };
            let set = |elements| Data::Frozenset { elements };
            let boolean_first = set(vec![
                Data::Boolean { value: true },
                Data::Integer { value: 1.into() },
            ]);
            let integer_first = set(vec![
                Data::Integer { value: 1.into() },
                Data::Boolean { value: true },
            ]);
            let first = bag.make_constant(boolean_first.borrow_constant());
            let second = bag.make_constant(integer_first.borrow_constant());
            assert!(!first.0.is(&second.0));
            assert!(
                first.0.downcast_ref::<PyFrozenSet>().unwrap().elements()[0]
                    .class()
                    .is(crate::builtins::PyBool::static_type())
            );
            assert!(
                second.0.downcast_ref::<PyFrozenSet>().unwrap().elements()[0]
                    .class()
                    .is(crate::builtins::PyInt::static_type())
            );
            let forward = set(vec![
                Data::Bytes {
                    value: b"first ordinary element".to_vec(),
                },
                Data::Bytes {
                    value: b"second ordinary element".to_vec(),
                },
            ]);
            let reverse = set(vec![
                Data::Bytes {
                    value: b"second ordinary element".to_vec(),
                },
                Data::Bytes {
                    value: b"first ordinary element".to_vec(),
                },
            ]);
            let forward = bag.make_constant(forward.borrow_constant());
            let reverse = bag.make_constant(reverse.borrow_constant());
            assert!(forward.0.is(&reverse.0));
            let snapshot = forward.0.downcast_ref::<PyFrozenSet>().unwrap().elements();
            let counts: Vec<_> = snapshot.iter().map(|value| value.strong_count()).collect();
            for _ in 0..3 {
                assert!(key(&forward.0).is_some());
                assert_eq!(
                    counts,
                    snapshot
                        .iter()
                        .map(|value| value.strong_count())
                        .collect::<Vec<_>>()
                );
            }
            drop(snapshot);
            let data = Data::Tuple {
                elements: vec![Data::Bytes {
                    value: b"private pool lifetime".to_vec(),
                }],
            };
            let value = bag.make_constant(data.borrow_constant());
            let again = bag.make_constant(data.borrow_constant());
            assert!(value.0.is(&again.0));
            drop(again);
            assert_eq!(value.0.strong_count(), 2);
            drop(pool);
            assert_eq!(value.0.strong_count(), 1);
            let child = &value
                .0
                .downcast_ref::<crate::builtins::PyTuple>()
                .unwrap()
                .as_slice()[0];
            assert_eq!(child.strong_count(), 1);
        });
    }

    #[cfg(feature = "compiler")]
    #[test]
    fn basic_route_finalizes_fields_before_publication() {
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let basic = crate::compiler::compile(
                "def left(x):\n return x + 12345678901234567890\ndef right(x):\n return x + 12345678901234567890\n",
                crate::compiler::Mode::Exec, "compile_cache.py", vm.compile_opts(),
            ).unwrap();
            let root = vm.new_code(basic);
            let nested: Vec<_> = root.code.constants.iter()
                .filter_map(|value| value.0.downcast_ref::<PyCode>()).collect();
            assert_eq!(nested.len(), 2);
            let [left, right] = [nested[0], nested[1]];
            assert!(!left.is(right));
            assert!(left.code.constants.tuple_owner().unwrap().0.is(&right.code.constants.tuple_owner().unwrap().0));
            assert!(left.linetable_bytes.is(&right.linetable_bytes));
            assert!(left.exceptiontable_bytes.is(&right.exceptiontable_bytes));
            assert!(left.names_tuple.is(&right.names_tuple));
            assert!(left.localsplus_names_tuple.is(&right.localsplus_names_tuple));
            assert_eq!(left.code.constants.tuple_owner().unwrap().0.as_object().strong_count(), 2);
            assert!(left.code_bytes.get().is_none());
            assert!(right.code_bytes.get().is_none());
            assert!(!left.co_code(vm).is(&right.co_code(vm)));
        });
    }
    #[cfg(feature = "compiler")]
    #[test]
    fn assembly_fields_share_literals_but_default_routes_do_not_pool() {
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let mut basic = crate::compiler::compile(
                "try:\n value = target\nexcept TypeError:\n value = None\n",
                crate::compiler::Mode::Exec,
                "compile_cache.py",
                vm.compile_opts(),
            )
            .unwrap();
            assert!(!basic.linetable.is_empty());
            assert!(!basic.exceptiontable.is_empty());
            basic.constants = vec![
                Data::Bytes {
                    value: basic.linetable.to_vec(),
                },
                Data::Bytes {
                    value: basic.exceptiontable.to_vec(),
                },
            ]
            .into_iter()
            .collect();
            let frozen = frozen::FrozenCodeObject::encode(&basic);
            let raw = basic.clone().map_bag(PyVmBag(vm));
            let literal_default = vm.new_code(raw);
            let frozen_default = vm.new_code(frozen);
            let pooled = vm.new_code(basic);
            for (index, field) in [&pooled.linetable_bytes, &pooled.exceptiontable_bytes]
                .into_iter()
                .enumerate()
            {
                assert!(pooled.code.constants.get(index).unwrap().0.is(field));
                assert_eq!(field.as_object().strong_count(), 2); // One tuple edge and one field edge.
            }
            assert!(
                !literal_default
                    .code
                    .constants
                    .first()
                    .unwrap()
                    .0
                    .is(&literal_default.linetable_bytes)
            );
            assert!(
                !frozen_default
                    .code
                    .constants
                    .first()
                    .unwrap()
                    .0
                    .is(&frozen_default.linetable_bytes)
            );
            let tuple = vm.ctx.new_tuple(vec![pooled.to_owned().into()]);
            assert!(key(pooled.as_object()).is_none());
            assert!(key(tuple.as_object()).is_none());
        });
    }

    #[cfg(feature = "compiler")]
    #[test]
    fn noncanonical_name_winner_is_not_published() {
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let basic = crate::compiler::compile(
                "result = target",
                crate::compiler::Mode::Exec,
                "compile_cache.py",
                vm.compile_opts(),
            )
            .unwrap();
            let raw = basic.map_bag(PyVmBag(vm));
            let noncanonical = vm.ctx.new_tuple(
                raw.names
                    .iter()
                    .map(|name| vm.ctx.new_str(name.as_str()).into())
                    .collect(),
            );
            assert!(!noncanonical.is_empty());
            for (value, canonical) in noncanonical.iter().zip(raw.names.iter()) {
                assert!(!value.is(*canonical));
            }
            let pool = Pool::default();
            let bag = CompileBag { vm, pool: &pool };
            let winner = bag.share(noncanonical.clone().into());
            assert!(winner.is(&noncanonical));
            drop(winner);
            let result = bag.finalize(raw);
            assert!(!result.names_tuple.is(&noncanonical));
            for (value, canonical) in result.names_tuple.iter().zip(result.code.names.iter()) {
                assert!(value.is(*canonical));
            }
            drop(pool);
            assert_eq!(noncanonical.as_object().strong_count(), 1);
        });
    }
}
