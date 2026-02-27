# Carvera Spindle Controller

Embassy Rust firmware for Pico 2 to interface between Carvera CNC and ESCON 50/5 motor controller.

## Commands

```bash
# Run unit tests (on host, not embedded target)
# macOS:
cargo test --lib --no-default-features --target x86_64-apple-darwin
# Linux:
cargo test --lib --no-default-features --target x86_64-unknown-linux-gnu

# Run linter (on host)
# macOS:
cargo clippy --lib --no-default-features --target x86_64-apple-darwin -- -D warnings
# Linux:
cargo clippy --lib --no-default-features --target x86_64-unknown-linux-gnu -- -D warnings

# Check formatting
cargo fmt -- --check

# Fix formatting
cargo fmt

# Build for RP2350 target
cargo build --release

# Verify binary has correct ImageDef (should show "RP2350" and "ARM Secure")
picotool info target/thumbv8m.main-none-eabihf/release/carvera_spindle -t elf

# Flash to device via picotool (hold BOOTSEL, connect USB first)
picotool load -u -v -x -t elf target/thumbv8m.main-none-eabihf/release/carvera_spindle
```

## Project Structure

- `src/lib.rs` - Pure functions and unit tests (testable on host)
- `src/main.rs` - Embedded firmware entry point (Embassy async)
- `WIRING.md` - Hardware wiring, ESCON configuration, and deployment guide
- `memory.x` - Linker script for RP2350
- `.cargo/config.toml` - Target and runner configuration

## Architecture

Simple single-loop design:
1. Read PWM input duty cycle via hardware counter (GPIO3)
2. Apply 5-sample moving average smoothing
3. Map 0-100% input to 10-90% output (GPIO4)
4. Set enable HIGH when input ≥10% (GPIO5)
5. Timeout to disabled state after 200ms no signal

## Testing

Unit tests run on host (not device) using `--no-default-features`:
- `calculate_output()` - PWM translation and enable logic
- `Smoother` - Moving average filter

## Stack Analysis (IMPORTANT - Check Before Every Change)

Embedded firmware has limited stack space (~8KB default). Stack overflows cause silent memory corruption that's extremely hard to debug. **Always analyze binary size and function sizes when modifying code.**

### Protection: flip-link

This project uses `flip-link` linker which places the stack at the start of RAM. Stack overflows cause a HardFault instead of silent corruption. This is configured in `.cargo/config.toml`.

### Analysis Commands (Run Before Every PR)

```bash
# Install tools (one-time setup)
cargo install flip-link cargo-bloat cargo-binutils
rustup component add llvm-tools-preview

# Build and check binary size
cargo build --release
cargo size --release --target thumbv8m.main-none-eabihf --bin carvera_spindle -- -A

# Find largest functions (large functions often have high stack usage)
cargo bloat --release --target thumbv8m.main-none-eabihf -n 40

# Check crate sizes to identify bloated dependencies
cargo bloat --release --target thumbv8m.main-none-eabihf --crates
```

### When to Check

- **Before every PR** - Run cargo-bloat to check for regressions
- After adding/modifying Embassy tasks
- After adding large local variables (arrays, structs)
- After adding new dependencies
- When debugging crashes or unexpected behavior

### Red Flags (Investigate These)

- Functions > 4KB in cargo-bloat output (likely high stack usage)
- Large local arrays/buffers in async functions
- Creating structs inside loops instead of reusing (move outside loop)
- Deep call chains in interrupt handlers
- New dependencies that increase .text size significantly

### Stack-Safe Patterns

```rust
// BAD: Creates PwmConfig on stack every iteration
loop {
    let mut config = PwmConfig::default();
    config.top = value;
    pwm.set_config(&config);
}

// GOOD: Reuse config allocated outside loop
let mut config = PwmConfig::default();
loop {
    config.top = value;
    pwm.set_config(&config);
}
```

### CI Integration

GitHub Actions runs binary size analysis on every PR:
- **Build** job: Compiles with flip-link for stack overflow protection
- **Size Analysis** job: Reports largest functions and crate sizes
- Check the "Binary Size Analysis" job output in PR checks

## Hardware

- Pico 2 (RP2350) with hardware FPU (single+double precision) on both Cortex-M33 cores
- 4 GPIO pins: PWM in (GPIO3), PWM out (GPIO4), Enable (GPIO5), Status LED (GPIO25)
- See WIRING.md for complete setup and deployment guide

Note: `f32::floor()`, `f32::round()`, `f32::abs()` require `libm` in `no_std`. Use manual
equivalents: truncation-to-int for floor (positive values), `+ 0.5` cast for round,
conditional for abs. `f32::clamp()` works in `no_std` (pure comparison, no libm).

## Status LED (GPIO25)

The onboard LED indicates firmware status:
- **Slow blink (~1Hz)**: Firmware running, no active PWM signal
- **Fast blink (~4Hz)**: Active PWM signal detected

## RP2350 Boot Requirements

The RP2350 requires an ImageDef block in flash for the boot ROM to recognize and execute firmware. This is different from RP2040 which used a BOOT2 stage.

### Critical: memory.x Sections

The `memory.x` linker script MUST include these sections for RP2350:

```
SECTIONS {
    .start_block : ALIGN(4) {
        __start_block_addr = .;
        KEEP(*(.start_block));
    } > FLASH
} INSERT AFTER .vector_table;
```

Without this, Embassy's `imagedef-secure-exe` feature creates the ImageDef but the linker doesn't place it correctly, causing the boot ROM to not recognize the firmware.

### Verifying Correct Build

Use picotool to verify the binary before flashing:
```bash
picotool info target/thumbv8m.main-none-eabihf/release/carvera_spindle -t elf
```

**Correct output:**
```
Program Information
 target chip:  RP2350
 image type:   ARM Secure
```

**Incorrect output (firmware won't boot):**
```
Family ID 'absolute'
```

### Flashing

Use picotool (not elf2uf2-rs, which doesn't properly support RP2350):
```bash
# Put Pico 2 in BOOTSEL mode: hold BOOTSEL button while connecting USB
picotool load -u -v -x -t elf target/thumbv8m.main-none-eabihf/release/carvera_spindle
```

Install picotool on macOS: `brew install picotool`

## Known probe-rs Issues with RP2350

### XIP Not Re-enabled After Flash Operations (probe-rs issue #3676)

If a flash download is interrupted or verification fails, probe-rs doesn't re-enable XIP (Execute In Place) mode. This causes:
- Flash reads return all zeros
- Flash verification fails even when the flash was written correctly
- Subsequent operations may fail with bus faults

**Workaround**: After flashing, trigger a system reset to re-enable XIP:
```bash
# Flash the firmware
probe-rs download --chip RP235x target/thumbv8m.main-none-eabihf/release/carvera_spindle

# Trigger system reset to re-enable XIP (write SYSRESETREQ to AIRCR)
probe-rs write --chip RP235x b32 0xe000ed0c 0x05FA0004

# Wait and verify flash is readable
sleep 1 && probe-rs read --chip RP235x b32 0x10000000 8
```

If flash reads still return zeros, the device may need a physical reset (power cycle or BOOTSEL button).

### Flash Verification Always Fails

picotool and probe-rs both report flash verification failures on RP2350 even when the flash is written correctly. This appears to be a timing/caching issue.

**Workaround**: Skip verification and check manually:
```bash
# Flash without verification
probe-rs download --chip RP235x firmware.elf  # no --verify flag

# Verify manually after reset
probe-rs write --chip RP235x b32 0xe000ed0c 0x05FA0004
sleep 1
probe-rs read --chip RP235x b32 0x10000000 8  # Should show vector table
```

## Related Repositories

The Carvera Smoothieware firmware is checked out at `../Carvera_Community_Firmware/` and can be accessed for reference when debugging PWM signal behavior or understanding Carvera's spindle control logic.

Key files:
- `src/modules/tools/spindle/AnalogSpindleControl.cpp` - Analog mode PWM output logic
- `src/modules/tools/spindle/PWMSpindleControl.cpp` - PWM mode (closed-loop) logic

## Git

- Never commit or push to git - the user will handle all git operations
