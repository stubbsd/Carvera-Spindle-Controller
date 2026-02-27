( Dump Calibration - Zigzag 3-tone sequence: 9000 -> 6000 -> 12000 RPM )
( Prints calibration table via defmt for debugging )
( Does not modify calibration data )

M3 S9000
G4 P1
M3 S6000
G4 P1
M3 S12000
G4 P1
M5
G4 P1

M5
M30
