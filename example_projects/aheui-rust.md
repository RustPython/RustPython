# aheui-rust

- Crate link: https://github.com/youknowone/aheui-rust/tree/main/rpaheui
- Creating a frozenlib: https://github.com/youknowone/aheui-rust/blob/main/rpaheui/src/lib.rs

This crate shows you how to embed an entire Python project into bytecode and ship it. Follow the `FROZEN` constant and see how it's made and how you can use it.

If you'd like to learn more about how to initialize the Standard Library with the `freeze-stdlib` feature, check out the example project at `example_projects/frozen_stdlib/src/main.rs`.

Just a heads-up, it doesn't automatically resolve dependencies. If you have more dependencies than the standard library, Don't forget to also freeze them.
