( Clear Calibration - Zigzag 3-tone sequence: 12000 -> 6000 -> 9000 RPM )
( Erases calibration data from flash and resets runtime correction )
( After clearing, run calibration_procedure.gcode to re-calibrate )

M3 S12000
G4 P1
M3 S6000
G4 P1
M3 S9000
G4 P1
M5
G4 P1

M5
M30
