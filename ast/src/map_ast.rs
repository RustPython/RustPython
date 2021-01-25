use std::convert::Infallible;

pub trait MapAst<T, U>: Sized {
    type Mapped;
    fn try_map_ast<E, F: FnMut(T) -> Result<U, E>>(self, f: &mut F) -> Result<Self::Mapped, E>;
    fn map_ast<F: FnMut(T) -> U>(self, mut f: F) -> Self::Mapped {
        let result: Result<_, Infallible> = self.try_map_ast(&mut |u| Ok(f(u)));
        match result {
            Ok(mapped) => mapped,
            Err(never) => match never {},
        }
    }
}

macro_rules! no_user {
    ($t:ty) => {
        impl<T, U> MapAst<T, U> for $t {
            type Mapped = Self;
            #[inline]
            fn try_map_ast<E, F: FnMut(T) -> Result<U, E>>(
                self,
                _f: &mut F,
            ) -> Result<Self::Mapped, E> {
                Ok(self)
            }
        }
    };
}

no_user!(String);
no_user!(crate::Constant);
no_user!(crate::ConversionFlag);
no_user!(bool);
no_user!(usize);

impl<T, U, A: MapAst<T, U>> MapAst<T, U> for crate::Located<A, T> {
    type Mapped = crate::Located<A::Mapped, U>;
    fn try_map_ast<E, F: FnMut(T) -> Result<U, E>>(self, f: &mut F) -> Result<Self::Mapped, E> {
        Ok(crate::Located {
            location: self.location,
            custom: f(self.custom)?,
            node: self.node.try_map_ast(f)?,
        })
    }
}

impl<T, U, A: MapAst<T, U>> MapAst<T, U> for Vec<A> {
    type Mapped = Vec<A::Mapped>;
    fn try_map_ast<E, F: FnMut(T) -> Result<U, E>>(self, f: &mut F) -> Result<Self::Mapped, E> {
        self.into_iter().map(|node| node.try_map_ast(f)).collect()
    }
}

impl<T, U, A: MapAst<T, U>> MapAst<T, U> for Option<A> {
    type Mapped = Option<A::Mapped>;
    fn try_map_ast<E, F: FnMut(T) -> Result<U, E>>(self, f: &mut F) -> Result<Self::Mapped, E> {
        self.map(|node| node.try_map_ast(f)).transpose()
    }
}

impl<T, U, A: MapAst<T, U>> MapAst<T, U> for Box<A> {
    type Mapped = Box<A::Mapped>;
    fn try_map_ast<E, F: FnMut(T) -> Result<U, E>>(self, f: &mut F) -> Result<Self::Mapped, E> {
        (*self).try_map_ast(f).map(Box::new)
    }
}
