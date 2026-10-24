# Runtime validation tooling

This directory contains optional Python tooling for preparing and running the
synthetic runtime-measurement workflow. It is not imported by the Rust crate,
the application, or its build/package scripts. The harness and tests use only
the Python standard library; application build and runtime remain Rust-only.

The synthetic source project is kept separately in
[`fixtures/synthetic-project/`](fixtures/synthetic-project/). It is a small
workflow fixture, not representative production data. The runner reads those
checked-in files and metadata as inputs; it does not regenerate or overwrite
them. A real measurement run writes its result manifest and logs only beneath
the selected output directory.

Run the harness unit tests from the repository root:

```sh
python3 -m unittest discover -s tools/runtime-validation -p 'test_*.py'
```

Inspect the CLI without starting a measurement run:

```sh
python3 tools/runtime-validation/runtime_validation.py --help
```

The real measurement mode requires prebuilt base and candidate binaries, the
pinned local model artifact, and an independently bounded memory and temporary
disk environment. See the harness help and the implementation plan for its
budgets and evidence requirements. No Python package installation is needed.
