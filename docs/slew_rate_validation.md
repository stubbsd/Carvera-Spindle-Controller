# Slew Rate Validation: ESCON 50/5 Handles Rapid Deceleration Without Software Limiter

## Purpose

Documents the empirical validation that the ESCON 50/5 servo controller handles rapid downward duty changes (deceleration) without regenerative braking overvoltage, eliminating the need for a software slew rate limiter.

## Background

When a brushless DC motor decelerates rapidly, it acts as a generator, feeding current back into the DC bus. If the bus voltage exceeds the controller's limit, an overvoltage fault (ESCON error 0x0001 "Vcc Overvoltage") triggers and the controller disables. A software slew rate limiter would artificially slow down duty decreases to prevent this.

## Test Methodology

A 10-step autonomous torture test was implemented:

- **Duty range**: 900 (90%) down to 100 (10%) = 800 unit total drop
- **Test progression**: Easiest to hardest, varying the number of 20ms ticks:
  - Test 1: 10 ticks (80 duty/tick, 200ms ramp)
  - Test 2: 9 ticks (88 duty/tick, 180ms ramp)
  - Test 3: 8 ticks (100 duty/tick, 160ms ramp)
  - Test 4: 7 ticks (114 duty/tick, 140ms ramp)
  - Test 5: 6 ticks (133 duty/tick, 120ms ramp)
  - Test 6: 5 ticks (160 duty/tick, 100ms ramp)
  - Test 7: 4 ticks (200 duty/tick, 80ms ramp)
  - Test 8: 3 ticks (266 duty/tick, 60ms ramp)
  - Test 9: 2 ticks (400 duty/tick, 40ms ramp)
  - Test 10: 1 tick (800 duty/tick, 20ms ramp) - full 800-unit drop in a single control tick
- **Per-test procedure**: Spin up to 90%, stabilize for 3s, ramp down, monitor ESCON alert pin for 500ms post-ramp, coast for 3s
- **Alert detection**: 1ms polling of ESCON digital alert output during and after each ramp

## Results

| Test | Duty/Tick | Ramp Time | Result |
|------|-----------|-----------|--------|
| 1 | 80 | 200ms | PASS |
| 2 | 88 | 180ms | PASS |
| 3 | 100 | 160ms | PASS |
| 4 | 114 | 140ms | PASS |
| 5 | 133 | 120ms | PASS |
| 6 | 160 | 100ms | PASS |
| 7 | 200 | 80ms | PASS |
| 8 | 266 | 60ms | PASS |
| 9 | 400 | 40ms | PASS |
| 10 | 800 | 20ms | PASS |

All 10 tests passed, including the most aggressive: an 800-unit duty drop (90% to 10%) in a single 20ms control tick. No ESCON alert was triggered at any point.

## Conclusion

The ESCON 50/5 handles the maximum possible deceleration rate without overvoltage faults. A software downward slew rate limiter is not needed for this hardware configuration (ESCON 50/5 + Carvera spindle motor + 24V PSU).

The likely explanation is that the ESCON's internal current controller and the system's electrical characteristics (motor inductance, bus capacitance, PSU absorption) are sufficient to handle regenerative energy from rapid deceleration.

## Hardware Configuration

- Controller: ESCON 50/5 servo controller
- Motor: Carvera stock brushless DC spindle motor
- PSU: 24V power supply
- Belt ratio: 1.635:1 (motor to spindle)
- Control loop: 50Hz (20ms tick)
