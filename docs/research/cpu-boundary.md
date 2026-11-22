# CPU boundary research

Microsoft's March 2020 Xbox Series X hardware disclosure identifies the shipped CPU family as custom AMD Zen 2. Class D applies to the vendor's hardware disclosure. Using x86-64 as the future guest ISA is based on that family identification. Platform-specific calling conventions and guest virtual-memory rules remain unknown. [Xbox Series X hardware disclosure](https://news.xbox.com/en-us/2020/03/16/xbox-series-x-tech/)

Apple documents the JIT mapping and write-protection APIs for Apple Silicon, including instructions for making newly written machine code visible to instruction fetch. Class B applies to the documented host APIs. Xenora has yet to choose ahead-of-time translation or JIT compilation. [Apple JIT porting guide](https://developer.apple.com/documentation/apple-silicon/porting-just-in-time-compilers-to-apple-silicon)

No guest page-size contract has been established. Host mapping operations use host page granularity. Any finer guest protection granularity will require an explicit software policy. Class B applies to documented host mapping semantics. [Apple mmap manual](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/mmap.2.html)

Future evaluation includes guest atomic and memory ordering, x87 and SSE numerical behavior, x86 vector mapping, guest scheduling and public system APIs. The graphics contract assumes no host TSO mode.
