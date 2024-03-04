# RoboPLC

An ultimate pack of tools for creating real-time micro-services, PLCs and
industrial-grade robots in Rust.

Note: the crate is actively developed. API can be changed at any time. Use at
your own risk!

RoboPLC is a part of [EVA ICS](https://www.eva-ics.com/) industrial
automation platform.

## DataBuffer

[`buf::DataBuffer`] covers a typical data exchange pattern when data
frames are collected (cached) by a single or multiple producers, then taken by
a single consumer in bulk and submitted, e.g. into a local database or into
an external bus.

<img
src="https://raw.githubusercontent.com/eva-ics/roboplc/main/schemas/databuffer.png"
width="350" />

* always has got a fixed capacity

* thread-safe out-of-the-box

* frames may be forcibly pushed, overriding the previous ones, like in a ring-buffer.
