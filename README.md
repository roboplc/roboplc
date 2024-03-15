# RoboPLC

An ultimate pack of tools for creating real-time micro-services, PLCs and
industrial-grade robots in Rust.

Note: the crate is actively developed. API can be changed at any time. Use at
your own risk!

RoboPLC is a part of [EVA ICS](https://www.eva-ics.com/) industrial
automation platform.

## Examples

Can be found at <https://github.com/eva-ics/roboplc/tree/main/examples>

## DataBuffer

[`buf::DataBuffer`] covers a typical data exchange pattern when data
frames are collected (cached) from a single or multiple producers, then taken
by a single consumer in bulk and submitted, e.g. into a local database or into
an external bus.

<img
src="https://raw.githubusercontent.com/eva-ics/roboplc/main/schemas/databuffer.png"
width="350" />

* always has got a fixed capacity

* thread-safe out-of-the-box

* frames may be forcibly pushed, overriding the previous ones, like in a ring-buffer.

## Hub

[`hub::Hub`] implements a data-hub (in process pub/sub) model, when multiple
clients (usually thread workers) exchange data via a single virtual bus instead
of using direct channels.

This brings some additional overhead into data exchange, however makes the
architecture significantly clearer, lowers code support costs and brings
additional features.

<img
src="https://raw.githubusercontent.com/eva-ics/roboplc/main/schemas/hub.png"
width="550" />

* classic pub/sub patterns with no data serialization overhead

* based on [`pchannel`] which allows to mix different kinds of data and apply
  additional policies if required

* a fully passive model with no "server" thread.

## pdeque and pchannel

A policy-based deque [`pdeque::Deque`] is a component to build policy-based
channels.

[`pchannel`] is a channel module, based on the policy-based deque.

Data policies supported:

* **Always** a frame is always delivered
* **Optional** a frame can be skipped if no room
* **Single** a frame must be delivered only once (the latest one)
* **SingleOptional** a frame must be delivered only once (the latest one) and
  is optional

Additionally, components support ordering by data priority and automatically
drop expired data if the data type has got an expiration marker method
implemented.

[`pchannel`] is a real-time safe channel, mean it may be not so fast as popular
channel implementations (it may be even slower than channels provided by
[`std::sync::mpsc`]). But it is **completely safe for real-time applications**,
mean there are no spin loops, data is always delivered with minimal latency and
threads do not block each other.

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
src="https://raw.githubusercontent.com/eva-ics/roboplc/main/schemas/controller.png"
width="550" />

## I/O

[`io`] module provides a set of tools to work with field devices and SCADA
buses.

Currently supported:

* Modbus (RTU/TCP) via [`io::modbus`]
  ([Modbus PLC example](https://github.com/eva-ics/roboplc/blob/main/examples/plc-modbus.rs))

* Raw UDP in/out via [`io::raw_udp`]
  ([Raw UDP in/out example](https://github.com/eva-ics/roboplc/blob/main/examples/raw-udp.rs))
