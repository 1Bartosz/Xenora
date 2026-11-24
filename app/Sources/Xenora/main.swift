// SPDX-License-Identifier: GPL-3.0-or-later
//
// Native shell over the versioned C interface. "Run trusted fixture" executes the
// bundled trusted PE fixture (x86-64 translated to ARM64; its draw request is rendered
// by native Metal) and shows the verified read-back frame. No other executable can run.

import CXenora
import Foundation
import SwiftUI

struct RuntimeProbe {
    var abiVersion: UInt32 = 0
    var status: Int32 = -1
    var sections: UInt32 = 0
    var entryRVA: UInt32 = 0

    var connected: Bool { abiVersion == XENORA_ABI_VERSION && status == XENORA_OK }

    static func run() -> RuntimeProbe {
        var probe = RuntimeProbe(abiVersion: xenora_abi_version())
        var session: xenora_session = 0
        probe.status = xenora_session_create(XENORA_ABI_VERSION, &session)
        guard probe.status == XENORA_OK else { return probe }
        defer { _ = xenora_session_destroy(session) }
        probe.status = xenora_session_inspect_trusted_fixture(session, &probe.sections, &probe.entryRVA)
        return probe
    }

    var summary: String {
        String(format: "abi=%u status=%d sections=%u entry_rva=0x%x", abiVersion, status, sections, entryRVA)
    }
}

/// The frame produced by one run of the trusted fixture.
struct GuestFrame {
    let status: Int32
    let width: Int
    let height: Int
    let rgba: [UInt8]

    static func run() -> GuestFrame {
        var session: xenora_session = 0
        var status = xenora_session_create(XENORA_ABI_VERSION, &session)
        guard status == XENORA_OK else { return GuestFrame(status: status, width: 0, height: 0, rgba: []) }
        defer { _ = xenora_session_destroy(session) }
        var rgba = [UInt8](repeating: 0, count: Int(XENORA_FIXTURE_FRAME_WIDTH * XENORA_FIXTURE_FRAME_HEIGHT * 4))
        var width: UInt32 = 0
        var height: UInt32 = 0
        status = rgba.withUnsafeMutableBufferPointer { buffer in
            xenora_session_run_trusted_fixture(session, buffer.baseAddress, UInt64(buffer.count), &width, &height)
        }
        return GuestFrame(status: status, width: Int(width), height: Int(height), rgba: status == XENORA_OK ? rgba : [])
    }

    var image: CGImage? {
        guard status == XENORA_OK, let provider = CGDataProvider(data: Data(rgba) as CFData) else { return nil }
        return CGImage(
            width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue),
            provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
    }

    var summary: String {
        let first = rgba.prefix(4).map(String.init).joined(separator: ",")
        return "status=\(status) frame=\(width)x\(height) first_pixel=\(first)"
    }
}

/// Owns the latest fixture run. A class with @Published avoids SwiftUI macros,
/// which the command-line toolchain does not ship.
@MainActor
final class FrameModel: ObservableObject {
    @Published var frame: GuestFrame?
    @Published var running = false

    func run() {
        running = true
        Task {
            // The fixture blocks while it executes and renders, so keep it off the main thread.
            frame = await Task.detached { GuestFrame.run() }.value
            running = false
        }
    }
}

struct ContentView: View {
    let probe = RuntimeProbe.run()
    @ObservedObject var model: FrameModel
    private var frame: GuestFrame? { model.frame }
    private var running: Bool { model.running }

    var body: some View {
        Form {
            Section("Guest display") {
                if let image = frame?.image {
                    Image(decorative: image, scale: 1)
                        .resizable()
                        .interpolation(.none)
                        .aspectRatio(contentMode: .fit)
                        .frame(maxWidth: .infinity, minHeight: 120, maxHeight: 240)
                        .accessibilityLabel("Guest frame, \(frame!.width) by \(frame!.height) pixels")
                    Text("Guest code ran, its draw was rendered by native Metal, and all \(frame!.width * frame!.height) pixels matched the fixture shader's output.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                } else if let frame {
                    Text("Run failed (status \(frame.status)). No frame is shown.")
                        .foregroundStyle(.red)
                }
                Button(running ? "Running…" : "Run trusted fixture") { model.run() }
                .disabled(running || !probe.connected)
            }
            LabeledContent("Runtime", value: probe.connected ? "Connected" : "Unavailable (status \(probe.status))")
            LabeledContent("Interface version", value: "\(probe.abiVersion)")
            LabeledContent("Trusted fixture sections", value: "\(probe.sections)")
            LabeledContent("Trusted fixture entry RVA", value: String(format: "0x%X", probe.entryRVA))
            Text("Only the bundled trusted fixture can run. Live presentation, persistent sessions and input are not available yet.")
                .foregroundStyle(.secondary)
        }
        .formStyle(.grouped)
        .frame(minWidth: 460, minHeight: 420)
    }
}

struct XenoraApp: App {
    let model = FrameModel()

    var body: some Scene {
        WindowGroup("Xenora") { ContentView(model: model) }
    }
}

// `--smoke-frame` runs the trusted fixture on the Metal device and exits; it needs a GPU.
if CommandLine.arguments.contains("--smoke-frame") {
    let frame = GuestFrame.run()
    print(frame.summary)
    exit(frame.status == XENORA_OK ? 0 : 1)
}

// `--smoke` prints the probe and exits without opening a window, for scripts and CI.
if CommandLine.arguments.contains("--smoke") {
    let probe = RuntimeProbe.run()
    print(probe.summary)
    exit(probe.connected ? 0 : 1)
}
XenoraApp.main()
