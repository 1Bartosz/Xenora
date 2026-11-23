# X1 bounded x86-64 interpreter

This isolated Cargo workspace implements the two original fixtures in the
[Phase 02 feasibility study](../../docs/research/phase02-cpu-feasibility.md).
Both fixtures use immutable code at guest address `0x1000` and an eight-byte
writable output at `0x2000`. The interpreter takes the end of the code slice
as the terminal address and validates each JNE target even when the condition
is false. The program prints initial state, each retired state and the terminal
state with output. The tests use literal expected RIP, RAX, arithmetic flags
and output bytes. Their independent review is pending.

The checked-in fixture bytes are staged separately from the Rust decoder.
`fixtures/check.sh` recreates the bytes from the assembly and prints tool
versions, architecture and linkage, fixture bytes and SHA-256 hashes. The
assembler emits a temporary x86-64 relocatable object and the script extracts
its text. The host runs the LLVM tools and Rust interpreter as ARM64 processes.

Run from the repository root after the active fuzz campaign completes.

```sh
set -e
set -o pipefail
manifest=experiments/x1/Cargo.toml
export CARGO_TARGET_DIR="$PWD/artifacts/x1/target"
export CARGO_BUILD_JOBS=2
mkdir -p artifacts/x1

LLVM_BIN="$(brew --prefix llvm)/bin" bash experiments/x1/fixtures/check.sh \
  2>&1 | tee artifacts/x1/fixtures.log
cargo fmt --manifest-path "$manifest" -- --check
cargo clippy --manifest-path "$manifest" --locked --target aarch64-apple-darwin \
  --all-targets -- -D warnings
cargo test --manifest-path "$manifest" --locked --target aarch64-apple-darwin \
  2>&1 | tee artifacts/x1/tests.log
cargo run --manifest-path "$manifest" --locked --target aarch64-apple-darwin \
  2>&1 | tee artifacts/x1/trace.log
driver="$CARGO_TARGET_DIR/aarch64-apple-darwin/debug/xenora-x1"
{ uname -m && sw_vers && file "$driver" && lipo -archs "$driver" && otool -L "$driver"; } \
  2>&1 | tee artifacts/x1/native.log
```

## X2 fixed-base PE32+ experiment

X2 interprets an original synthetic AMD64 guest image. The loader uses the
header, directory and section layouts from the [Microsoft PE format specification](https://learn.microsoft.com/en-us/windows/win32/debug/pe-format).
It accepts a narrow profile with a fixed image base of `0x140000000`, a
512-byte header page and two disjoint 512-byte raw section pages. Guest
addresses translate to the declared virtual bytes of immutable RX code at
`0x140001000` and eight writable data bytes at `0x140002000`. Raw alignment
padding is outside both guest ranges. The entry must be the start of code.
The image must declare stripped relocations and all 16 data directories
must be empty. Arbitrary Windows executables and game execution are outside
this experiment's scope.

The initial state sets RCX to 41, RDI to the data address, RFLAGS to 2 and
all other registers to zero. The guest loads service ID 1 into EAX. In X2,
`UD2` dispatches the synthetic service selected by RAX. Service 1 adds one
to RCX modulo 2⁶⁴ and writes the result to RAX, preserving RCX and RFLAGS.
It resumes two bytes after the trap. An unknown ID faults at the trap RIP
before retirement. The final instruction stores RAX to the data range.
Execution ends at the first byte past the declared code range.

| Event | RIP after event | RAX | RCX |
| --- | --- | --- | --- |
| Entry | `0x140001000` | 0 | 41 |
| Load service ID | `0x140001005` | 1 | 41 |
| Return from trap | `0x140001007` | 42 | 41 |
| Store and finish | `0x14000100a` | 42 | 41 |

Run from the repository root.

```sh
python3 experiments/x1/fixtures/x2.py --check
export CARGO_BUILD_JOBS=2
export CARGO_TARGET_DIR="$PWD/artifacts/x2/target"
cargo test --manifest-path experiments/x1/Cargo.toml --locked --target aarch64-apple-darwin
cargo run --manifest-path experiments/x1/Cargo.toml --locked --target aarch64-apple-darwin -- x2
```

## X3 fixed-base guest compute experiment

X3 uses an original AMD64 PE32+ fixture with fixed image base `0x400000`.
The shared X2 loader validates the PE entry and maps the declared virtual
bytes of RX code at `0x401000`, eight writable status bytes at `0x402000`,
and 48 read-only bytes at `0x403000`. The lower image base keeps the command
address representable by the interpreter's existing `MOV EAX, imm32`.
The read-only section holds the 32-byte command followed by the 16-byte
input backing handle 1. The PE initializes status to eight `0xa5` bytes.
The host initializes RDI to the status address. The X1 interpreter starts
at the validated PE entry. Guest code loads the command address into EAX,
traps with `UD2`, then stores RAX through RDI.

The command contains eight little-endian 32-bit fields in order.

| Field | Fixture value | Validation |
| --- | --- | --- |
| Version | 1 | Must be 1 |
| Service ID | 2 | Must be 2 |
| Kernel ID | 1 | Selects the original trusted OR kernel |
| Input handle | 1 | Selects the fixture's 16-byte input |
| Access mode | 1 | Read-only |
| Input byte offset | 4 | Word aligned and within the selected input |
| Input byte length | 12 | Must equal four times the grid word count |
| Grid word count | 3 | Range 1 through 4 |

The trap checks the full command address and input range before it hands
owned input words to Metal. Each invocation ORs one input word with
`0x00ff00ff`. `NativeCompute::run_words` waits for command completion and
returns owned readback. Success returns zero in RAX for the guest to store.
Malformed commands and backend failures fault at the trap without retiring
it. The guest status retains its initial bytes in those cases. The host
compares the completed readback with three literal expected words. This
protocol is local to X3 and covers synthetic compute only.

The default build has no Metal dependency. Run from the repository root
with an external wall-clock limit around the Metal invocation.

```sh
export CARGO_BUILD_JOBS=2
export CARGO_TARGET_DIR="$PWD/artifacts/x3/target"
manifest=experiments/x1/Cargo.toml
target=aarch64-apple-darwin
python3 experiments/x1/fixtures/x3.py --check
cargo test --manifest-path "$manifest" --locked --target "$target" -- --nocapture
cargo build --manifest-path "$manifest" --locked --target "$target" --features metal
env MTL_DEBUG_LAYER=1 MTL_SHADER_VALIDATION=1 MTL_SHADER_VALIDATION_REPORT_TO_STDERR=1 \
  MTL_SHADER_VALIDATION_ABORT_ON_FAULT=1 \
  python3 -c 'import subprocess, sys; sys.exit(subprocess.run(sys.argv[1:], timeout=30).returncode)' \
  "$CARGO_TARGET_DIR/$target/debug/xenora-x1" x3
```

## X4 fixed-base guest render experiment

Two original PE fixtures use the X3 image layout and the X1 trap path.
Each carries a 32-byte command followed by 16 read-only parameter bytes.
The little-endian command fields are version (1), service (3), target handle
(1), width (8), height (8), format (1 for RGBA8Unorm), parameter offset,
and parameter length (4). The offset is relative to the parameter
area and must select an aligned, in-bounds 32-bit value in 0–255.
Handle 1 identifies the sole 8×8 target for that invocation. The guest
status stays pending if validation or Metal submission fails.

The experiment uses the existing native offscreen test's Metal command
and texture pattern in its own graphics module. A full-screen triangle
writes a single shared texture. After Metal completion, the runner reads
level zero and compares all 256 bytes with a row-major CPU expectation
of `[x, y, p, 255]`. The two fixtures supply `p=17` and `p=93`.
The run prints capabilities, completion, guest status, and readback bytes.
This covers a synthetic fixture only. Game execution is unsupported.

Run from the repository root.

```sh
export CARGO_BUILD_JOBS=2
export CARGO_TARGET_DIR="$PWD/artifacts/x4/target"
export PYTHONDONTWRITEBYTECODE=1
manifest=experiments/x1/Cargo.toml
target=aarch64-apple-darwin
python3 experiments/x1/fixtures/x4.py --check
cargo test --manifest-path "$manifest" --locked --target "$target" --features metal
cargo build --manifest-path "$manifest" --locked --target "$target" --features metal
env MTL_DEBUG_LAYER=1 MTL_SHADER_VALIDATION=1 MTL_SHADER_VALIDATION_REPORT_TO_STDERR=1 \
  MTL_SHADER_VALIDATION_ABORT_ON_FAULT=1 \
  python3 -c 'import subprocess, sys; sys.exit(subprocess.run(sys.argv[1:], timeout=30).returncode)' \
  "$CARGO_TARGET_DIR/$target/debug/xenora-x1" x4
```
