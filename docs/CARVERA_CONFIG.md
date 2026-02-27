# Carvera Spindle Configuration for ESCON Controller

This firmware is designed for **open-loop PWM mode** operation with Carvera. In open-loop
mode, Carvera sends PWM proportional to the requested speed without closed-loop PID
correction, while still monitoring RPM feedback for display and alarm detection.
The Pico scales the 0-100% input to 10-90% ESCON output.

> **Note**: This requires the Carvera Community Firmware with the `open_loop` feature
> added to PWMSpindleControl. See the "PWM Open-Loop Mode" section below.

## Switching to PWM Open-Loop Mode (Recommended)

Run these commands in the Carvera Controller's G-code console (MDI):

### Step 1: Set Open-Loop PWM Configuration

```gcode
config-set sd spindle.type pwm
config-set sd spindle.open_loop true
config-set sd spindle.max_rpm 20437
config-set sd spindle.pwm_period 50
config-set sd spindle.alarm_pin p2.11!
config-set sd spindle.pulses_per_rev 4
config-set sd spindle.acc_ratio 1.635
```

**Key settings:**
- `spindle.type pwm` - Use PWM spindle control (has alarm and RPM feedback support)
- `spindle.open_loop true` - Bypass PID, use direct PWM scaling like analog mode
- `spindle.pulses_per_rev 4` - ESCON outputs 4 PPR (wired directly to Carvera)
- `spindle.acc_ratio 1.635` - Ratio to convert motor RPM to spindle RPM for display

### Step 2: Reset the Machine

After running the config-set commands, you MUST reset the Carvera for changes to take effect:
- Power cycle the machine, OR
- Use the Controller software reset function

### Step 3: Verify Settings

After reset, verify the configuration:

```gcode
config-get sd spindle.type
config-get sd spindle.open_loop
config-get sd spindle.max_rpm
config-get sd spindle.pwm_period
config-get sd spindle.alarm_pin
config-get sd spindle.pulses_per_rev
config-get sd spindle.acc_ratio
```

Expected output:
- `spindle.type = pwm`
- `spindle.open_loop = true`
- `spindle.max_rpm = 20437`
- `spindle.pwm_period = 50` (20kHz: 1,000,000 / 50 = 20,000 Hz)
- `spindle.alarm_pin = p2.11!`
- `spindle.pulses_per_rev = 4`
- `spindle.acc_ratio = 1.635`

### Step 4: Test Spindle

```gcode
M3 S6000    ; Start spindle at 6000 RPM
            ; Verify Pico display shows correct speed
M5          ; Stop spindle
```

---

## Restoring Default PWM Mode (Closed-Loop PID)

To revert to factory closed-loop PWM control with PID:

### Run in G-code Console:

```gcode
config-set sd spindle.type pwm
config-set sd spindle.open_loop false
config-set sd spindle.max_rpm 15000
config-set sd spindle.pwm_period 1000
config-set sd spindle.pulses_per_rev 12
config-set sd spindle.acc_ratio 1.635
```

### Reset Machine

Power cycle or software reset required.

### Verify Defaults:

```gcode
config-get sd spindle.type
config-get sd spindle.open_loop
config-get sd spindle.max_rpm
config-get sd spindle.pwm_period
config-get sd spindle.pulses_per_rev
config-get sd spindle.acc_ratio
```

Expected:
- `spindle.type = pwm`
- `spindle.open_loop = false`
- `spindle.max_rpm = 15000`
- `spindle.pwm_period = 1000` (1kHz)
- `spindle.pulses_per_rev = 12`
- `spindle.acc_ratio = 1.635`

---

## Configuration Reference

| Setting | Open-Loop (Recommended) | Closed-Loop PID (Default) | Description |
|---------|-------------------------|---------------------------|-------------|
| spindle.type | pwm | pwm | Control mode |
| spindle.open_loop | true | false | Bypass PID, use direct PWM scaling |
| spindle.pwm_period | 50 | 1000 | PWM period in microseconds |
| spindle.max_rpm | 20437 | 15000 | Max RPM for PWM scaling |
| spindle.alarm_pin | p2.11! | nc | Alert input pin (! = active low) |
| spindle.pulses_per_rev | 4 | 12 | Encoder pulses per revolution (ESCON outputs 4 PPR) |
| spindle.acc_ratio | 1.635 | 1.635 | Ratio - converts motor RPM to spindle RPM for display |

### PWM Period to Frequency Conversion

Frequency (Hz) = 1,000,000 / period (microseconds)

| Period | Frequency |
|--------|-----------|
| 50 | 20 kHz |
| 100 | 10 kHz |
| 1000 | 1 kHz (default) |

### max_rpm Calculation

Open-loop mode uses LINEAR PWM formula: `duty = (S / max_rpm) × max_pwm`

The Pico scales this to ESCON's 10-90% range, and ESCON interprets it as:
`motor_rpm = (duty - 10%) / 80% × 12500`

For correct mapping: `max_rpm = motor_max × ratio = 12500 × 1.635 = 20437`

With this setting, S10000 produces:
- Carvera duty: (10000 / 20437) × 100% = 48.9%
- Pico scales to ESCON: maps 48.9% input to appropriate 10-90% output
- ESCON motor: ~6116 RPM
- Spindle: 6116 × 1.635 = ~10000 RPM ✓

---

## Why Open-Loop Mode?

Carvera's default PWM mode uses closed-loop PID to control spindle speed.
When combined with the ESCON motor controller (which has its own closed-loop control),
this creates a "double closed-loop" situation that can cause issues:

1. **Feedback conflicts**: Both Carvera and ESCON try to adjust speed independently
2. **Start/stop cycling**: Carvera may detect speed mismatch during motor acceleration
3. **PID tuning conflicts**: Two independent control loops fighting each other

**Open-loop mode** (`spindle.open_loop true`) is the best solution because it:
- Sends PWM proportional to requested speed (like analog mode)
- Keeps alarm pin monitoring (halts machine on spindle fault)
- Keeps RPM feedback display in Carvera UI
- Lets ESCON handle all speed regulation internally

This is better than pure analog mode (`spindle.type analog`) which lacks alarm and
RPM display support entirely.

---

## Firmware Adjustments for 20kHz PWM

The Pico firmware is configured for 20kHz PWM input (open-loop mode):

- **PIO clock**: 18.75MHz (150MHz / 8) for adequate resolution at 20kHz
- **Averaging window**: 640 cycles (~32ms) for stable readings
- **Resolution**: ~0.1% duty cycle accuracy

These settings are in `src/tasks/pwm_input.rs` and `src/state.rs`.

---

## Alarm Pin (Stall Detection)

The Pico firmware outputs an alert signal when it detects a spindle stall or fault.
With open-loop PWM mode, Carvera monitors this pin and halts the machine if triggered.

**Wiring:**
- Pico GPIO10 (Error output) → Carvera P2.11 input
- Alert is active-LOW: normally HIGH, goes LOW on fault

**Behavior:**
1. Pico detects stall (actual RPM << target RPM while current is high)
2. Pico pulls alert line LOW
3. Carvera's `get_alarm()` sees the LOW signal (inverted by `!` in config)
4. Carvera halts with "ALARM: Spindle alarm triggered"

This feature requires:
- Carvera Community Firmware with `open_loop` support in PWMSpindleControl
- `spindle.alarm_pin p2.11!` configured (the `!` inverts the active-low signal)

---

## Troubleshooting

### Spindle not responding after config change
- Ensure you reset the machine after config-set commands
- Verify settings with config-get commands

### Speed display incorrect in Carvera UI
The RPM shown in Carvera depends on two settings:
- `spindle.pulses_per_rev` - must match ESCON output (4 PPR)
- `spindle.acc_ratio` - ratio to convert motor RPM to spindle RPM (1.635)

**Common issue**: If `pulses_per_rev = 12` (default) but ESCON outputs 4 PPR, Carvera
waits for 12 pulses (3 motor revolutions) before updating, causing incorrect readings.

**Fix**: `config-set sd spindle.pulses_per_rev 4`

With correct settings, Carvera displays: `motor_rpm × acc_ratio = spindle_rpm`

### M957 command (diagnostic)
Run `M957` to see current spindle state and PWM values (if available on your Carvera version).

### Firmware verification
The Pico LCD displays:
- **Requested RPM**: Speed calculated from Carvera's PWM input
- **Actual RPM**: Speed measured from ESCON encoder output
- **Current**: Motor current from ESCON analog output
