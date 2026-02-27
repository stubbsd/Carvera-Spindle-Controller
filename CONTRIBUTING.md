# Contributing to Carvera Spindle Controller

Thanks for your interest in contributing! This project is part of the Carvera CNC community, and contributions are welcome.

## Ways to Contribute

- **Bug reports**: Open an issue describing the problem, expected behavior, and steps to reproduce
- **Feature requests**: Open an issue describing what you'd like to see
- **Documentation**: Improve README, add examples, fix typos
- **Code**: Fix bugs, add features, improve tests

## Development Setup

### Prerequisites

- Rust toolchain (1.82+): `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
- RP2350 target: `rustup target add thumbv8m.main-none-eabihf`
- picotool (for flashing): `brew install picotool` (macOS) or build from source

### Clone and Build

```bash
git clone https://github.com/Cloud-Badger/Carvera-Spindle-Controller.git
cd Carvera-Spindle-Controller

# Build for embedded target
cargo build --release

# Run tests (on host)
cargo test --lib --no-default-features --target x86_64-apple-darwin  # macOS
cargo test --lib --no-default-features --target x86_64-unknown-linux-gnu  # Linux
```

### Running Lints

```bash
# Check formatting
cargo fmt -- --check

# Run clippy (on host)
cargo clippy --lib --no-default-features --target x86_64-apple-darwin -- -D warnings
```

## Code Structure

```
src/
├── lib.rs              # Re-exports, integration tests (testable on host)
├── main.rs             # Embedded entry point, task spawning
├── adc.rs              # ADC-to-current conversion
├── calibration.rs      # Calibration sequence detection, recording, interpolation
├── conversion.rs       # PWM/RPM/duty conversion functions
├── display.rs          # Display data types (ErrorType, DisplayStatus)
├── filters.rs          # CircularBuffer for signal processing
├── flash_store.rs      # Flash serialization/deserialization for calibration data
├── lcd.rs              # HD44780 LCD driver with RGB backlight
├── speed.rs            # Period-to-RPM conversion, median filtering
├── stabilization.rs    # Speed stabilization tracking
├── stall.rs            # Stall detection with dynamic grace periods
├── state.rs            # Shared atomics, centralized error system, config constants
├── temperature.rs      # ADC-to-temperature conversion (RP2350 internal sensor)
├── threshold.rs        # Generic threshold detector with debounce
└── tasks/              # Embassy async tasks (embedded feature only)
    ├── mod.rs           # Task module exports
    ├── calibration.rs   # Calibration orchestrator (sequence + flash I/O)
    ├── current_monitor.rs  # ADC current reading + overcurrent detection
    ├── lcd.rs           # LCD rendering task
    ├── led.rs           # Status LED blink patterns
    ├── pwm_input.rs     # PIO-based PWM input measurement
    ├── speed_measure.rs # PIO-based speed measurement (encoder)
    ├── spindle_control.rs  # Main control loop (50 Hz)
    ├── thermal.rs       # MCU temperature monitoring
    └── watchdog.rs      # Hardware watchdog + heartbeat health checks
```

For detailed architecture documentation including task map, data flow diagrams, and shared state inventory, see [ARCHITECTURE.md](ARCHITECTURE.md).

### Key Design Decisions

1. **Separation of concerns**: Pure functions in `lib.rs` modules are testable without hardware. Embassy tasks in `tasks/` handle hardware interaction.

2. **Configuration in one place**: All tunable parameters are in `src/state.rs` under the `config` module. See [docs/CONFIGURATION.md](docs/CONFIGURATION.md) for a guide.

3. **Inter-task communication**: Uses Embassy `Watch` channel for display updates and `AtomicBool`/`AtomicU32` for real-time control flags.

4. **Centralized error system**: Per-source error flags with priority-based arbitration in `state.rs`. Overcurrent and thermal errors permanently latch; stall and ESCON alert follow their source state.

5. **Async LCD**: LCD operations use async delays to avoid blocking other tasks.

## Submitting Changes

1. Fork the repository
2. Create a feature branch: `git checkout -b my-feature`
3. Make your changes
4. Ensure tests pass: `cargo test --lib --no-default-features`
5. Ensure clippy is happy: `cargo clippy --lib --no-default-features`
6. Ensure formatting: `cargo fmt`
7. Commit with a clear message
8. Push and open a pull request

## Configuration Parameters

If you're adding new configuration options, add them to `src/state.rs` in the `config` module with:

- Clear documentation comment explaining what it does
- Sensible default value
- Units in the comment (e.g., "in milliseconds", "in 0.1% units")

## Testing on Hardware

If you have a Pico 2 and the hardware setup:

1. Build: `cargo build --release`
2. Put Pico in BOOTSEL mode (hold BOOTSEL while connecting USB)
3. Flash: `picotool load -u -v -x -t elf target/thumbv8m.main-none-eabihf/release/carvera_spindle`

## Related Projects

- [Carvera Community](https://github.com/Carvera-Community) - Community firmware and tools
- [Instructables Spindle Upgrade](https://www.instructables.com/Carvera-Spindle-Power-Upgrade-Stock-Motor/) - Simpler DFR1036 approach

## Questions?

- Open an issue for project-related questions
- Join [r/carvera](https://www.reddit.com/r/carvera/) for general Carvera discussion

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
