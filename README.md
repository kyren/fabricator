# Fabricator of Merriments
---

A WIP replacement for the GameMaker Studio compiler and runtime, developed for and used by [Fields
of Mistria](https://www.fieldsofmistria.com/).

This project is a bit different than other GameMaker runtime replacements in that it reads GMS2
project files and compiles and runs GML directly, rather than running bytecode compiled by GMS2. As
such, it can have a totally different compiler and VM design than GMS2. Notably, the VM here is more
similar to Lua's VM -- it is register based rather than stack based and designed for multiple return
values, heap allocated closures, and coroutines.

There is a working Compiler and VM for GML that compiles GML faster than GameMaker's VM mode and
runs code faster than GameMaker's YYC mode. GML is extended with enough features to make it a much
nicer, more modern language, and the stricter FML subset removes many confusing features and warts.

The VM is written in almost entirely safe Rust and provides an extremely rich, safe FFI to Rust to
replace GameMaker's very limited DLL extension FFI. For example, the VMs normal FFI interface is
used to implement the entirety of the GML stdlib and project runner.

Compatibility with GML (the language) is very good and the compiler and VM are fairly complete.
The non-game-abstraction `stdlib` standard library crate is *somewhat* complete. The actual project
*runner* (the `fabricator` crate) which contains the higher level parts of GameMaker such as the
room / object / instance abstraction, graphics, input, and sound is very much a WIP. It should be
noted however, that the runner is not a requirement to use this project! Notably it is possible
to port a GameMaker game to use *only* the compiler, VM, and stdlib, replacing GameMaker's builtin
abstractions with custom ones written in a custom runner.

The project is written to be compatible *enough* with GML and GameMaker to make it possible to port
a large, complex project, but is explicitly *not* aiming for perfect bug-for-bug compatibility. The
project aims for compatibility without compromising a reasonable specification and implementation --
it does NOT aim to reimplement unambiguous and avoidable GML or GMS2 bugs or mimic what the author
considers serious design flaws in GML semantics.

This project also does NOT intend to be a drop-in replacement for GameMaker Studio or a replacement
for the GameMaker Studio IDE in any way, it only allows running existing projects using an alternate
compiler and runtime. It will primarily be useful to projects that are straining at the boundaries
of what can be accomplished in vanilla GameMaker Studio and would like to move away from GameMaker's
abstractions or integrate new, complex APIs to custom engine code.

## License

Everything in this repository is licensed under any of:

* MIT license [LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT
* MIT-0 license [LICENSE-MIT-0](LICENSE-MIT-0) or http://opensource.org/licenses/MIT-0
* Creative Commons CC0 1.0 Universal Public Domain Dedication [LICENSE-CC0](LICENSE-CC0)
  or https://creativecommons.org/publicdomain/zero/1.0/

at your option.
