# Changelog

## RoboPLC

### 0.4.0 (2024-07-29)

* Custom real-time locking policies

* [RTSC](https://crates.io/crates/rtsc) 0.3 integration

* Stability and architecture improvements

* [RFlow](https://crates.io/crates/rflow) integration

* Docker support

### 0.3.0 (2024-06-16)

* Real-time-safe data synchronization components moved to
  [RTSC](https://crates.io/crates/rtsc) crate.

### 0.2.0 (2024-05-09)

* Re-exported locking primitives are re-exported as `locking`

* Modbus server write access control

### 0.1.49 (2024-05-07)

* Added subprocess pipe I/O

### 0.1.48 (2024-04-24)

* Locking primitives have been switched to a real-time fork of `parking_lot`

### 0.1 (2024-04-15)

* First public release

## RoboPLC CLI

### 0.1.21 (2024-06-18)

* Added `restart` command
* Added `build-custom` configuration section

### 0.1 (2024-04-15)

* First public release

## RoboPLC manager

### 0.2 (2024-06-22)

* The default configuration file has been switched to TOML format.

* [RVideo](https://crates.io/crates/rvideo) video streams preview.

* Minor improvements and bug fixes.

### 0.1 (2024-04-15)

* First public release
