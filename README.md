# COSMIC Edit
COSMIC Text Editor

Currently an incomplete **pre-alpha**, this project is a work in progress - issues are expected.

## Testing
You can test by installing a current version of Rust and then building with `cargo`.

```SHELL
git clone https://github.com/pop-os/cosmic-edit
cd cosmic-edit
cargo build
```

You can get more detailed errors by using the `RUST_LOG` environment variables, that you can invoke for just that one command like this: `RUST_LOG=debug cargo run`. This will give you more detail about the application state. You can go even futher with `RUST_LOG=trace cargo run`, that shows all logging details about the applicaiton.

## Clippy Lints
PRs are welcome, as it builds a better product for everyone. It is recomended that you check your code with Clippy Lints turned on. You can find more about [Configuring Clippy](https://doc.rust-lang.org/nightly/clippy/configuration.html) here.