<h2>
  RoboPLC
  <a href="https://crates.io/crates/roboplc"><img alt="crates.io page" src="https://img.shields.io/crates/v/roboplc.svg"></img></a>
  <a href="https://docs.rs/roboplc"><img alt="docs.rs page" src="https://docs.rs/roboplc/badge.svg"></img></a>
</h2>

<img src="https://raw.githubusercontent.com/roboplc/roboplc/main/roboplcline_.png"
width="200" />

[RoboPLC](https://www.bohemia-automation.com/software/roboplc/) is an ultimate
pack of a framework and tools for creating real-time micro-services, PLCs and
industrial-grade robots in Rust.

The crate is designed to let using all its components both separately and
together.

RoboPLC is a part of [EVA ICS](https://www.eva-ics.com/) industrial
automation platform.

Real-time-safe data synchronization components are re-exported from the
[RTSC](https://docs.rs/rtsc) crate which is a part of RoboPLC project and can
be used directly, with no requirement to use RoboPLC.

## Technical documentation

Available at <https://info.bma.ai/en/actual/roboplc/index.html>

## Examples

Can be found at <https://github.com/roboplc/roboplc/tree/main/examples>

## DataBuffer

[`buf::DataBuffer`] covers a typical data exchange pattern when data
frames are collected (cached) from a single or multiple producers, then taken
by a single consumer in bulk and submitted, e.g. into a local database or into
an external bus.

<img
src="https://raw.githubusercontent.com/roboplc/roboplc/main/schemas/databuffer.png"
width="350" />

* always has got a fixed capacity

* thread-safe out-of-the-box

* frames may be forcibly pushed, overriding the previous ones, like in a ring-buffer.

## Hub

[`hub::Hub`] implements a data-hub (in-process pub/sub) model, when multiple
clients (usually thread workers) exchange data via a single virtual bus instead
of using direct channels.

This brings some additional overhead into data exchange, however makes the
architecture significantly clearer, lowers code support costs and brings
additional features.

<img
src="https://raw.githubusercontent.com/roboplc/roboplc/main/schemas/hub.png"
width="550" />

* classic pub/sub patterns with no data serialization overhead

* based on [`policy_channel`] which allows to mix different kinds of data and
  apply additional policies if required

* a fully passive model with no "server" thread.

## pdeque and policy_channel

A policy-based deque [`rtsc::pdeque::Deque`] is a component to build policy-based
channels.

[`policy_channel`] is a channel module, based on the policy-based deque.

Data policies supported:

* **Always** a frame is always delivered
* **Latest** a frame is always delivered, previous are dropped if no room
  (acts like a ring-buffer)
* **Optional** a frame can be skipped if no room
* **Single** a frame must be delivered only once (the latest one)
* **SingleOptional** a frame must be delivered only once (the latest one) and
  is optional

Additionally, components support ordering by data priority and automatically
drop expired data if the data type has got an expiration marker method
implemented.

[`policy_channel`] is a real-time safe channel, mean it may be not so fast as
popular channel implementations (it may be even slower than channels provided
by [`std::sync::mpsc`]). But it is **completely safe for real-time
applications**, mean there are no spin loops, data is always delivered with
minimal latency and threads do not block each other.

## Real-time

[`thread_rt::Builder`] provides a thread builder component, which extends the
standard thread builder with real-time capabilities: scheduler policies and CPU
affinity (Linux only).

[`supervisor::Supervisor`] provides a lightweight task supervisor to manage
launched threads.

## Controller

[`controller::Controller`] is the primary component of mixing up all the
functionality together.

<img
src="https://raw.githubusercontent.com/roboplc/roboplc/main/schemas/controller.png"
width="550" />

## I/O

[`io`] module provides a set of tools to work with field devices and SCADA
buses.

Currently supported:

* Modbus (RTU/TCP) via [`io::modbus`] ([Modbus client/master
  example](https://github.com/roboplc/roboplc/blob/main/examples/modbus-master.rs),
  [Modbus server/slave
  example](https://github.com/roboplc/roboplc/blob/main/examples/modbus-slave.rs)),
  requires `modbus` crate feature.

* Raw UDP in/out via [`io::raw_udp`]
  ([Raw UDP in/out example](https://github.com/roboplc/roboplc/blob/main/examples/raw-udp.rs))

* Subprocess pipes via [`io::pipe`]
  ([Subprocess pipe example](https://github.com/roboplc/roboplc/blob/main/examples/pipe.rs))

* [EVA ICS](https://www.eva-ics.com/) EAPI in/out via [`io::eapi`] ([EVA ICS
  example](https://github.com/roboplc/roboplc/blob/main/examples/eapi.rs)),
  requires `eapi` crate feature

* SNMP v1/2/3 via [`snmp2`](https://crates.io/crates/snmp2) external crate.

* [ADS](https://crates.io/crates/roboplc-io-ads) connector for [Beckhoff
  TwinCAT](https://infosys.beckhoff.com/english.php?content=../content/1033/tcinfosys3/11291871243.html&id=),
  requires a license for commercial use

* [IEC 60870-5](https://crates.io/crates/roboplc-io-iec60870-5) client,
  requires a license for commercial use

## Locking safety

Note: the asynchronous components use `parking_lot_rt` locking only.

By default, the crate uses [parking_lot](https://crates.io/crates/parking_lot)
for locking. For real-time applications, the following features are available:

* `locking-rt` - use [parking_lot_rt](https://crates.io/crates/parking_lot_rt)
  crate which is a spin-free fork of parking_lot.

* `locking-rt-safe` - use [RTSC](https://crates.io/crates/rtsc)
  priority-inheritance locking, which is not affected by priority inversion
  (Linux only, recommended Kernel 5.14+).

Note: to switch locking policy, disable the crate default features.

The locking policy can be also selected in CLI when creating a new project:

```shell
robo new --locking rt-safe # the default for CLI-created projects is rt-safe
```

## Using on other platforms

The components [`thread_rt`], [`supervisor`] and [`controller`] can work on
Linux machines only.

## Migration from 0.3.x

* `pchannel` and `pchannel_async` have been renamed to [`policy_channel`] and
  [`policy_channel_async`] respectively.

* By default, the crate uses
  [parking_lot](https://crates.io/crates/parking_lot) for locking. To switch to
  more safe real-time locking, disable the crate default features and enable
  either `locking-rt` or `locking-rt-safe`. **This is important for real-time
  applications and must be enabled manually**.

* As [RTSC](https://crates.io/crates/rtsc) components are lock-agnostic, which
  requires to specify generic locking types, the modules [`channel`],
  [`policy_channel`], [`buf`] and [`semaphore`] are now wrappers around RTSC
  modules with the chosen locking policy.

* [`hub_async`] now requires `async` feature to be enabled.

## MSRV

Minimum supported mainstream Rust version of RoboPLC is synchronized with the
[Ferrocene](https://ferrocene.dev/) Rust compiler. This allows to create
mission-critical software, compliant with ISO 26262 (TCL 3/ASIL D), IEC 61508
(T3) and IEC 62304.

Current MSRV: mainstream 1.81.0, Ferrocene 24.11.0. Certain features may work
with older Rust versions.
