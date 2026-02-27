# Architecture

Overview of the Carvera Spindle Controller firmware architecture.

## Task Map

The firmware runs as multiple concurrent Embassy async tasks on the RP2350 Cortex-M33:

| Task | File | Frequency | Responsibility |
|------|------|-----------|----------------|
| `spindle_control_task` | `tasks/spindle_control.rs` | 50 Hz (20ms) | PWM output, enable/disable, stall detection, error GPIO, display updates |
| `pwm_input_task` | `tasks/pwm_input.rs` | PIO-driven (~32ms batches) | Read Carvera PWM input via PIO, 640-cycle averaging, publish MEASURED_DUTY |
| `speed_measure_task` | `tasks/speed_measure.rs` | Edge-driven | Measure ESCON speed output (4 PPR), publish MEASURED_RPM and MEASURED_FREQ_MHZ |
| `current_monitor_task` | `tasks/current_monitor.rs` | 10 Hz (100ms) | ADC current reading, overcurrent detection, temperature reading (every 50th cycle) |
| `calibration_task` | `tasks/calibration.rs` | 50 Hz (20ms) | Sequence detection, calibration recording, flash read/write |
| `lcd_task` | `tasks/lcd.rs` | ~5 Hz (200ms) | HD44780 LCD rendering, RGB backlight control |
| `led_task` | `tasks/led.rs` | 1-4 Hz | Status LED blink patterns (slow=idle, fast=active) |
| `watchdog_task` | `tasks/watchdog.rs` | 1 Hz | Hardware watchdog feeding, task heartbeat health checks |
| `thermal_monitor_task` | `tasks/thermal.rs` | 0.2 Hz (5s) | MCU temperature monitoring, thermal shutdown |

## Data Flow

```
Carvera CNC                    Pico 2 (RP2350)                     ESCON 50/5
-----------                    ---------------                     ----------

PWM 20kHz -----> [PIO] -----> MEASURED_DUTY (AtomicU32)
  (GPIO3)        640-cycle          |
                 averaging          |
                                    v
                         [spindle_control_task]
                            |           |
                 correct_duty()    stall detection
                            |           |
                            v           v
                     PWM 1kHz out   error_out GPIO10 ----> Carvera alarm input
                      (GPIO4)
                            |
                            +-----> enable_pin GPIO5 ----> ESCON DIN1

ESCON speed out -----> [PIO] -----> MEASURED_RPM (AtomicU32)
  (4 PPR, GPIO9)       period         |
                       measurement     +----> LCD display (actual RPM)
                                       +----> Stall detector (comparison)

ESCON analog out ----> [ADC] -----> CURRENT_MA (AtomicU32)
  (GPIO26)              |               |
                        +----> overcurrent detection
                        +----> LCD display (current)

ESCON alert ---------> [GPIO8] ----> spindle_control_task
  (active-low)                        (report_escon_alert)

                ESCON DOUT ---------> Carvera encoder input (4 PPR)
                  (direct wire, bypasses Pico)
```

## Shared State Inventory

All inter-task state lives in `src/state.rs` using lock-free atomics:

| Variable | Type | Writer(s) | Reader(s) | Purpose |
|----------|------|-----------|-----------|---------|
| `MEASURED_DUTY` | `AtomicU32` | pwm_input | spindle_control, calibration | Raw PWM input duty (0-10000) |
| `MEASURED_RPM` | `AtomicU32` | speed_measure | spindle_control, lcd | Actual motor RPM from encoder |
| `MEASURED_FREQ_MHZ` | `AtomicU32` | speed_measure | spindle_control | Raw frequency for diagnostics |
| `CURRENT_MA` | `AtomicU32` | current_monitor | spindle_control, lcd | Motor current in milliamps |
| `ENABLED` | `AtomicBool` | spindle_control | lcd, led | Spindle enable state |
| `CAL_SEQUENCE_ACTIVE` | `AtomicBool` | calibration | spindle_control, calibration | Suppresses stall detection during calibration |
| `DISPLAY_DATA` | Watch channel | spindle_control | lcd | Display status snapshot |
| `CAL_STATUS` | Watch channel | calibration | lcd | Calibration progress for LCD |

### Centralized Error System

Error state is managed by dedicated functions in `state.rs` with per-source flags and priority-based arbitration:

| Function | Behavior |
|----------|----------|
| `report_overcurrent()` | Sets OVERCURRENT_ERROR + SAFETY_SHUTDOWN (permanent latch) |
| `report_thermal()` | Sets THERMAL_ERROR + SAFETY_SHUTDOWN (permanent latch) |
| `report_stall_alert(active)` | Follows StallDetector state (not permanently latched) |
| `report_stall_latched(latched)` | Visual latch for display (shows "Stall" vs "StallCleared") |
| `report_escon_alert(active)` | Follows GPIO pin state (not latched) |
| `is_safety_shutdown()` | True if overcurrent or thermal has fired (only power cycle clears) |
| `any_error_active()` | OR of all four error sources |
| `get_active_error_type()` | Highest-priority active error (Overcurrent > Thermal > Stall > EsconAlert > None) |

When `any_error_active()` is true, `spindle_control_task` forces the enable pin LOW and PWM output to zero regardless of requested speed.

## Module Dependency Graph

```
lib.rs (re-exports)
  |
  +-- calibration.rs     depends on: state (config, CAL_SEQUENCE_ACTIVE)
  +-- conversion.rs      pure functions (no dependencies)
  +-- display.rs         pure types (no dependencies)
  +-- filters.rs         pure types (no dependencies)
  +-- flash_store.rs     depends on: calibration (types)
  +-- lcd.rs             depends on: display, calibration, state (config)
  +-- speed.rs           pure functions (no dependencies)
  +-- stabilization.rs   pure functions (no dependencies)
  +-- stall.rs           depends on: state (config)
  +-- state.rs           depends on: display (ErrorType)
  +-- temperature.rs     pure functions (no dependencies)
  +-- threshold.rs       pure functions (no dependencies)
  +-- adc.rs             depends on: state (config)
  |
  +-- tasks/ (embedded only)
       +-- spindle_control.rs  orchestrator: reads atomics, writes PWM/GPIO
       +-- pwm_input.rs        PIO program + averaging
       +-- speed_measure.rs    PIO period measurement
       +-- current_monitor.rs  ADC + overcurrent threshold
       +-- calibration.rs      sequence detection + flash I/O
       +-- lcd.rs              display rendering
       +-- led.rs              status LED
       +-- watchdog.rs         hardware watchdog
       +-- thermal.rs          temperature monitoring
```

## Calibration System

The calibration table maps measured PWM duty cycles to true RPM across 386 speed steps (750-20000 RPM in 50 RPM increments). Points are stored as packed `AtomicU32` values (`rpm << 16 | duty`) for lock-free access.

Runtime correction uses O(1) index lookup into the evenly-spaced table followed by piecewise-linear interpolation within the bracket. See `calibration.rs` for the full algorithm.

## Latency Budget

| Stage | Typical | Worst Case |
|-------|---------|------------|
| PIO PWM input averaging | 16ms | 32ms |
| Control loop period | 0ms | 20ms |
| PWM output update | <1us | <1us |
| **Total input-to-output** | **~26ms** | **~52ms** |
