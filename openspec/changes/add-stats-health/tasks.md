# Tasks: add-stats-health

## 1. Monitor (`crates/estream-health`)
- [x] 1.1 Sample types; counters → rates with reset handling; 5-minute history
- [x] 1.2 Rules with hysteresis (render lag, encoder, network, reconnecting, CPU, GPU, disk)
- [x] 1.3 System sampler (CPU, memory, GPU via PDH, disk) on a background thread

## 2. Studio
- [x] 2.1 Gather counters each second; status bar health dot; toasts for new alerts
- [x] 2.2 Stats dialog: tiles with sparklines, encoders and destinations, alerts with advice
- [x] 2.3 Remote API `GetStats`: CPU, memory and disk values

## 3. Verification
- [x] 3.1 Monitor and rule unit tests
- [x] 3.2 System sampler test
- [x] 3.3 In-app check with an overloaded encoder
