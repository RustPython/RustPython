use NativeType;
use std::rc::Rc;
use std::ops::Deref;

pub fn print(args: Vec<Rc<NativeType>>) -> NativeType {
    for elem in args {
        // TODO: figure out how python's print vectors
        match elem.deref() {
            &NativeType::NoneType => println!("None"),
            &NativeType::Boolean(ref b)=> {
                if *b {
                    println!("True");
                } else {
                    println!("False");
                }
            },
            &NativeType::Int(ref x)  => println!("{}", x),
            &NativeType::Float(ref x)  => println!("{}", x),
            &NativeType::Str(ref x)  => println!("{}", x),
            &NativeType::Unicode(ref x)  => println!("{}", x),
            _ => panic!("Print for {:?} not implemented yet", elem),
            /*
            List(Vec<NativeType>),
            Tuple(Vec<NativeType>),
            Iter(Vec<NativeType>), // TODO: use Iterator instead
            Code(PyCodeObject),
            Function(Function),
            #[serde(skip_serializing, skip_deserializing)]
            NativeFunction(fn(Vec<NativeType>) -> NativeType ),
            */
        }
    }
    NativeType::NoneType
}

pub fn len(args: Vec<Rc<NativeType>>) -> NativeType {
    if args.len() != 1 {
        panic!("len(s) expects exactly one parameter");
    }
    let len = match args[0].deref() {
        &NativeType::List(ref l) => l.borrow().len(),
        &NativeType::Tuple(ref t) => t.len(),
        &NativeType::Str(ref s) => s.len(),
        _ => panic!("TypeError: object of this type has no len()")
    };
    NativeType::Int(len as i32)
}
