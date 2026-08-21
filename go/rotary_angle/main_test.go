package main

import "testing"

func intPtr(v int) *int { return &v }

func TestShouldPublish(t *testing.T) {
	tests := []struct {
		name      string
		published *int
		adc       int
		want      bool
	}{
		{"first reading always publishes", nil, 2048, true},
		{"below deadband stays quiet", intPtr(2048), 2058, false},
		{"deadband reached publishes", intPtr(2048), 2073, true},
		{"deadband crossed downwards", intPtr(2048), 2020, true},
		{"end of travel pinned at 0", intPtr(10), 0, true},
		{"end of travel pinned at full scale", intPtr(4090), 4095, true},
		{"resting at an end stays quiet", intPtr(0), 0, false},
	}
	for _, tt := range tests {
		if got := shouldPublish(tt.published, tt.adc, 4095, 25); got != tt.want {
			t.Errorf("%s: shouldPublish(%v, %d) = %v, want %v",
				tt.name, tt.published, tt.adc, got, tt.want)
		}
	}
}

func TestDefaultDeadband(t *testing.T) {
	// ~0.6 % of full scale, rounded up, never below one count.
	if got := defaultDeadband(4095); got != 25 {
		t.Errorf("defaultDeadband(4095) = %d, want 25", got)
	}
	if got := defaultDeadband(1023); got != 7 {
		t.Errorf("defaultDeadband(1023) = %d, want 7", got)
	}
	if got := defaultDeadband(1); got != 1 {
		t.Errorf("defaultDeadband(1) = %d, want 1", got)
	}
}

func TestPayloadFor(t *testing.T) {
	sensor := &Sensor{adcMax: 4095, angleMax: 300.0}
	payload := payloadFor(sensor, 2048)
	if payload["adc"] != 2048 || payload["adcMax"] != 4095 {
		t.Errorf("unexpected counts in %v", payload)
	}
	// 2048 / 4095 * 300 = 150.04... -> rounded to one decimal
	if payload["angle"] != 150.0 {
		t.Errorf("angle = %v, want 150.0", payload["angle"])
	}
	if payload["angleMax"] != 300.0 {
		t.Errorf("angleMax = %v, want 300.0", payload["angleMax"])
	}
	// The ends map exactly onto the mechanical travel.
	if p := payloadFor(sensor, 4095); p["angle"] != 300.0 {
		t.Errorf("angle at full scale = %v, want 300.0", p["angle"])
	}
	if p := payloadFor(sensor, 0); p["angle"] != 0.0 {
		t.Errorf("angle at zero = %v, want 0.0", p["angle"])
	}
}
