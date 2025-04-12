# Changelog

## RoboPLC

### 0.6.0 (2025-04-12)

* EAPI actions must now return a serializable value. The value is used for
  `lmacro` and `unit` action outputs

* HMI support out-of-the-box

* Minor improvements and bug fixes

### 0.5.0 (2025-01-11)

* State load/save

* Atomic timers

* Removed unnecessary dependencies, lightweight build

* System tools and thread scheduler moved to
  [`rtsc`](https://crates.io/crates/rtsc) crate, musl support (experimental).

* MSRV set to 1.81.0

* Program live updates (Pro version)

* Program rollback (Pro version)

### 0.4.4 (2024-08-27)

* Added metrics-exporter-scope

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

## RoboPLC manager

### 0.6 (2025-04-12)

* New advanced terminal for remote program execution.

### 0.5 (2025-01-11)

* Professional version

* Live update (requires RoboPLC Pro)

* Program rollback (requires RoboPLC Pro)

* Minor UI improvements

### 0.4 (2024-07-29)

* Added [RFlow](https://crates.io/crates/rflow) support

* Added Docker support

* Added remote command execution

### 0.2 (2024-06-22)

* The default configuration file has been switched to TOML format.

* [RVideo](https://crates.io/crates/rvideo) video streams preview.

* Minor improvements and bug fixes.

### 0.1 (2024-04-15)

* First public release

## RoboPLC CLI

### 0.6.0 (2025-04-12)

* `metrics` command to display exported metrics in CLI.

* Ability to work directly on the host where RoboPLC manager is running.

* New advanced terminal support for remote program execution.

### 0.5.0 (2025-01-11)

* New features from RoboPLC 0.5

### 0.4 (2024-07-29)

* Added `rflow` support

* Fixed versions

* Starting from the version 0.4, the CLI version is synchronized with the
  RoboPLC version

### 0.1.21 (2024-06-18)

* Added `restart` command
* Added `build-custom` configuration section

### 0.1 (2024-04-15)

* First public release
