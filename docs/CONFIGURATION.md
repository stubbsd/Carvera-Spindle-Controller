# Configuration Guide

All firmware configuration is done at compile time via constants in `src/state.rs`. There is no runtime configuration interface. After changing any value, rebuild and reflash the firmware.

## Priority 1: Motor and Speed Configuration

These must match your ESCON Studio settings and Carvera configuration.

### `MAX_RPM` (default: 12500)

Location: `state::config::MAX_RPM`

The motor RPM at 90% ESCON duty cycle. Must match the "Speed at 90%" value configured in ESCON Studio.

```rust
pub const MAX_RPM: u32 = 12500;
```

### `CARVERA_SPINDLE_MAX_RPM` (default: 20437)

Location: `state::config::CARVERA_SPINDLE_MAX_RPM`

Maximum spindle RPM, computed from `MAX_RPM * BELT_RATIO_X1000 / 1000`. Used to convert between Carvera's PWM duty cycle and actual RPM. Must match Carvera firmware config `spindle.max_rpm` (default 15000; set to 20437 when using open-loop ESCON mode).

For the stock 1.635:1 belt ratio: `12500 * 1635 / 1000 = 20437`

Set in Carvera via: `config-set sd spindle.max_rpm 20437`

```rust
pub const CARVERA_SPINDLE_MAX_RPM: u32 = MAX_RPM * BELT_RATIO_X1000 / 1000;
```

### `CURRENT_AT_3V3_MA` (default: 5200)

Location: `state::config::CURRENT_AT_3V3_MA`

The current in milliamps that corresponds to 3.3V on the ESCON analog output. Must match the ESCON Studio analog output scaling.

For a 5.2A motor at full scale: 5200 mA

```rust
pub const CURRENT_AT_3V3_MA: u32 = 5200;
```

### `MIN_ENABLE_RPM` (default: 750)

Location: `state::config::MIN_ENABLE_RPM`

Minimum requested RPM before the spindle enable output is asserted. Prevents spindle creep from Carvera's idle PWM offset (~2.44% duty = S500).

Set this above Carvera's idle offset but below the lowest speed you intend to use.

```rust
pub const MIN_ENABLE_RPM: u32 = 750;
```

### `MIN_RPM` (default: 0)

Location: `state::config::MIN_RPM`

The motor RPM at 10% ESCON duty cycle. Set to 0 for full range, or higher if your motor does not run well below a certain speed. Must match ESCON Studio "Speed at 10%" setting.

```rust
pub const MIN_RPM: u32 = 0;
```

## Priority 2: Belt Ratio and Encoder

### `BELT_RATIO_X1000` (default: 1635)

Location: `state::config::BELT_RATIO_X1000`

Belt drive ratio from motor to spindle, multiplied by 1000 for integer math. The Carvera stock ratio is 1.635:1 (from `spindle.acc_ratio` in config.default).

Motor RPM * 1635 / 1000 = Spindle RPM

```rust
pub const BELT_RATIO_X1000: u32 = 1635;
```

### `ESCON_PULSES_PER_REV` (default: 4)

Location: `state::config::ESCON_PULSES_PER_REV`

Number of encoder pulses per motor revolution from the ESCON speed output. The ESCON 50/5 outputs 4 pulses per revolution by default.

```rust
pub const ESCON_PULSES_PER_REV: u32 = 4;
```

## Priority 3: Safety Thresholds

### Overcurrent

- `OVERCURRENT_THRESHOLD_PCT` (default: 90) - Percentage of `CURRENT_AT_3V3_MA` that triggers overcurrent
- `OVERCURRENT_DEBOUNCE_MS` (default: 50) - Current must exceed threshold for this duration

### Stall Detection

- `STALL_THRESHOLD_PCT` (default: 30) - Stall if actual RPM < this percentage of requested
- `STALL_DEBOUNCE_MS` (default: 100) - Stall condition must persist for this duration
- `STALL_RECOVERY_MS` (default: 300) - Speed must stay above threshold for this duration to clear
- `BASE_GRACE_MS` (default: 200) - Grace period after speed change before stall detection
- `RPM_GRACE_FACTOR` (default: 15) - Additional grace ms per 1000 RPM of speed change

### Thermal

- `THERMAL_SHUTDOWN_C` (default: 70) - MCU temperature limit in Celsius

## Priority 4: Pin Assignments

GPIO pin assignments for different boards are defined in `state::pins`. The defaults are configured for the standard Pico 2 wiring:

| Function | GPIO | Constant |
|----------|------|----------|
| PWM Input (from Carvera) | 3 | `pins::PWM_INPUT` |
| PWM Output (to ESCON) | 4 | `pins::PWM_OUTPUT` |
| Enable Output (to ESCON) | 5 | `pins::ENABLE` |
| ESCON Alert Input | 8 | `pins::ESCON_ALERT` |
| Speed Input (from ESCON) | 9 | `pins::SPEED_INPUT` |
| Error Output (to Carvera) | 10 | `pins::ERROR_OUTPUT` |
| Status LED | 25 | `pins::STATUS_LED` |
| Current ADC | 26 | `pins::CURRENT_ADC` |
| LCD RS | 16 | `pins::LCD_RS` |
| LCD E | 17 | `pins::LCD_E` |
| LCD D4-D7 | 18, 22, 20, 21 | `pins::LCD_D4` - `pins::LCD_D7` |
| LCD RGB Backlight | 14, 12, 13 | `pins::LCD_RED/GREEN/BLUE` |

To change pin assignments for a different board layout, edit the constants in the `pins` module and update `main.rs` peripheral initialization to match.

## Compile-Time Configuration Approach

All configuration uses Rust `const` values rather than runtime settings. This means:

- **No flash wear** from configuration changes
- **Zero runtime overhead** (constants are inlined at compile time)
- **Type-safe** (compiler catches mismatched types)
- **No configuration corruption** risk

The tradeoff is that changing configuration requires rebuilding and reflashing. For a CNC motor controller where configuration changes are rare and correctness is critical, this is the preferred approach.
