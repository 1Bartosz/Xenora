// SPDX-License-Identifier: GPL-3.0-or-later
//
// Native shell over the versioned C interface. "Run trusted fixture" executes the
// bundled trusted PE fixture (x86-64 translated to ARM64; its draw request is rendered
// by native Metal) and shows the verified read-back frame. No other executable can run.

import AppKit
import AVFoundation
import CXenora
import Foundation
import SwiftUI
import UniformTypeIdentifiers

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

/// The frame produced by running the textured fixture (Gate 5, #279).
struct TexturedFrame {
    let status: Int32
    let width: Int
    let height: Int
    let rgba: [UInt8]

    static func run() -> TexturedFrame {
        var session: xenora_session = 0
        var status = xenora_session_create(XENORA_ABI_VERSION, &session)
        guard status == XENORA_OK else { return TexturedFrame(status: status, width: 0, height: 0, rgba: []) }
        defer { _ = xenora_session_destroy(session) }
        var rgba = [UInt8](repeating: 0, count: 16)
        var width: UInt32 = 0
        var height: UInt32 = 0
        status = rgba.withUnsafeMutableBufferPointer { buffer in
            xenora_session_run_textured_fixture(session, buffer.baseAddress, UInt64(buffer.count), &width, &height)
        }
        return TexturedFrame(status: status, width: Int(width), height: Int(height), rgba: status == XENORA_OK ? rgba : [])
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
        let pixels = rgba.map(String.init).joined(separator: ",")
        return "status=\(status) frame=\(width)x\(height) pixels=[\(pixels)]"
    }
}

/// The frame and audio status produced by running the audio/VFS fixture (Gate 6, #284).
struct AudioVfsFrame {
    let status: Int32
    let width: Int
    let height: Int
    let samples: UInt32
    let totalAudioBytes: UInt64
    let rgba: [UInt8]
    let pcm: [UInt8]

    static func run(captureAudio: Bool = false) -> AudioVfsFrame {
        var session: xenora_session = 0
        var status = xenora_session_create(XENORA_ABI_VERSION, &session)
        guard status == XENORA_OK else {
            return AudioVfsFrame(status: status, width: 0, height: 0, samples: 0, totalAudioBytes: 0, rgba: [], pcm: [])
        }
        defer { _ = xenora_session_destroy(session) }
        var rgba = [UInt8](repeating: 0, count: 16)
        var width: UInt32 = 0
        var height: UInt32 = 0
        var samples: UInt32 = 0
        status = rgba.withUnsafeMutableBufferPointer { buffer in
            xenora_session_run_audio_vfs_fixture(
                session, buffer.baseAddress, UInt64(buffer.count), &width, &height, &samples)
        }
        var queued: UInt32 = 0
        var underruns: UInt32 = 0
        var total: UInt64 = 0
        _ = xenora_session_audio_status(session, &queued, &underruns, &total)
        var pcm: [UInt8] = []
        if captureAudio && status == XENORA_OK && queued > 0 && queued <= 256 * 1024 {
            pcm = [UInt8](repeating: 0, count: Int(queued))
            var read: UInt64 = 0
            let readStatus = pcm.withUnsafeMutableBufferPointer { buffer in
                xenora_session_audio_read(session, buffer.baseAddress, UInt64(buffer.count), &read)
            }
            if readStatus != XENORA_OK || read != UInt64(queued) {
                pcm.removeAll()
            }
        }
        return AudioVfsFrame(
            status: status,
            width: Int(width),
            height: Int(height),
            samples: samples,
            totalAudioBytes: total,
            rgba: status == XENORA_OK ? rgba : [],
            pcm: pcm
        )
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
        let pixels = rgba.map(String.init).joined(separator: ",")
        return "status=\(status) frame=\(width)x\(height) samples=\(samples) audio_total=\(totalAudioBytes) pixels=[\(pixels)]"
    }
}

private enum FixturePlaybackError: Error {
    case invalidPCM
    case timedOut
}

private func playFixturePCM(_ pcm: [UInt8]) throws -> Int {
    // The embedded audio/VFS fixture submits mono 48 kHz S16LE frames.
    guard !pcm.isEmpty, pcm.count.isMultiple(of: 2),
          let format = AVAudioFormat(standardFormatWithSampleRate: 48_000, channels: 1),
          let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(pcm.count / 2)),
          let samples = buffer.floatChannelData?[0] else {
        throw FixturePlaybackError.invalidPCM
    }
    buffer.frameLength = buffer.frameCapacity
    for frame in 0..<Int(buffer.frameLength) {
        let low = UInt16(pcm[frame * 2])
        let high = UInt16(pcm[frame * 2 + 1]) << 8
        samples[frame] = Float(Int16(bitPattern: low | high)) / 32_768
    }
    let engine = AVAudioEngine()
    let player = AVAudioPlayerNode()
    let played = DispatchSemaphore(value: 0)
    engine.attach(player)
    engine.connect(player, to: engine.mainMixerNode, format: format)
    defer {
        player.stop()
        engine.stop()
        engine.detach(player)
    }
    try engine.start()
    player.scheduleBuffer(buffer, completionCallbackType: .dataPlayedBack) { _ in
        played.signal()
    }
    player.play()
    guard played.wait(timeout: .now() + .seconds(3)) == .success else {
        throw FixturePlaybackError.timedOut
    }
    return Int(buffer.frameLength)
}

/// Owns the latest fixture run. A class with @Published avoids SwiftUI macros,
/// which the command-line toolchain does not ship.
@MainActor
final class FrameModel: ObservableObject {
    @Published var frame: GuestFrame?
    @Published var running = false
    @Published var texturedFrame: TexturedFrame?
    @Published var runningTextured = false
    @Published var audioVfsFrame: AudioVfsFrame?
    @Published var runningAudioVfs = false

    func run() {
        running = true
        Task {
            // The fixture blocks while it executes and renders, so keep it off the main thread.
            frame = await Task.detached { GuestFrame.run() }.value
            running = false
        }
    }

    func runTextured() {
        runningTextured = true
        Task {
            texturedFrame = await Task.detached { TexturedFrame.run() }.value
            runningTextured = false
        }
    }

    func runAudioVfs() {
        runningAudioVfs = true
        Task {
            audioVfsFrame = await Task.detached { AudioVfsFrame.run() }.value
            runningAudioVfs = false
        }
    }
}

/// Persistent guest rendering model (Gate 3, #266; Gate 4, #276).
@MainActor
final class FrameLoopModel: ObservableObject {
    @Published var state: UInt32 = UInt32(XENORA_LOOP_NONE)
    @Published var completed: UInt32 = 0
    @Published var frameId: UInt32 = XENORA_NO_FRAME
    @Published var image: CGImage?
    @Published var errorMessage: String?
    @Published var isInteractive: Bool = false

    private var session: xenora_session = 0
    private var pollTask: Task<Void, Never>?

    deinit {
        pollTask?.cancel()
        if session != 0 {
            _ = xenora_session_destroy(session)
        }
    }

    var isRunning: Bool { state == UInt32(XENORA_LOOP_RUNNING) }
    var isPaused: Bool { state == UInt32(XENORA_LOOP_PAUSED) }
    var canStart: Bool {
        session == 0 || state == UInt32(XENORA_LOOP_NONE) || state == UInt32(XENORA_LOOP_FINISHED)
            || state == UInt32(XENORA_LOOP_STOPPED) || state == UInt32(XENORA_LOOP_FAILED)
    }

    var stateDescription: String {
        switch state {
        case UInt32(XENORA_LOOP_NONE): return "Idle"
        case UInt32(XENORA_LOOP_RUNNING): return "Running"
        case UInt32(XENORA_LOOP_PAUSED): return "Paused"
        case UInt32(XENORA_LOOP_FINISHED): return "Finished"
        case UInt32(XENORA_LOOP_STOPPED): return "Stopped"
        case UInt32(XENORA_LOOP_FAILED): return "Failed"
        default: return "Unknown (\(state))"
        }
    }

    func start(frameCount: UInt32 = 300) {
        errorMessage = nil
        if session != 0 {
            _ = xenora_session_destroy(session)
            session = 0
        }
        var newSession: xenora_session = 0
        let createStatus = xenora_session_create(XENORA_ABI_VERSION, &newSession)
        guard createStatus == XENORA_OK else {
            errorMessage = "Failed to create session (status \(createStatus))"
            return
        }
        session = newSession
        let startStatus = xenora_session_start_frame_loop(session, frameCount)
        guard startStatus == XENORA_OK else {
            errorMessage = "Failed to start frame loop (status \(startStatus))"
            _ = xenora_session_destroy(session)
            session = 0
            return
        }
        isInteractive = false
        state = UInt32(XENORA_LOOP_RUNNING)
        completed = 0
        frameId = XENORA_NO_FRAME
        image = nil
        startPolling()
    }

    func startInteractive(frameCount: UInt32 = 300) {
        errorMessage = nil
        if session != 0 {
            _ = xenora_session_destroy(session)
            session = 0
        }
        var newSession: xenora_session = 0
        let createStatus = xenora_session_create(XENORA_ABI_VERSION, &newSession)
        guard createStatus == XENORA_OK else {
            errorMessage = "Failed to create session (status \(createStatus))"
            return
        }
        session = newSession
        let startStatus = xenora_session_start_interactive_loop(session, frameCount)
        guard startStatus == XENORA_OK else {
            errorMessage = "Failed to start interactive loop (status \(startStatus))"
            _ = xenora_session_destroy(session)
            session = 0
            return
        }
        isInteractive = true
        state = UInt32(XENORA_LOOP_RUNNING)
        completed = 0
        frameId = XENORA_NO_FRAME
        image = nil
        startPolling()
    }

    func sendInput(buttons: UInt32) {
        guard session != 0 else { return }
        var input = xenora_input_state(
            buttons: buttons,
            stick_x: 0,
            stick_y: 0,
            mouse_dx: 0,
            mouse_dy: 0,
            mouse_buttons: 0,
            connected: 1,
            reserved: 0
        )
        _ = xenora_session_send_input(session, &input)
    }

    func pause() {
        guard session != 0 else { return }
        _ = xenora_session_pause_frame_loop(session)
    }

    func resume() {
        guard session != 0 else { return }
        _ = xenora_session_resume_frame_loop(session)
    }

    func stop() {
        guard session != 0 else { return }
        _ = xenora_session_stop_frame_loop(session)
    }

    private func startPolling() {
        pollTask?.cancel()
        pollTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 16_000_000)
                guard let self = self, self.session != 0 else { break }
                var st: UInt32 = 0
                var comp: UInt32 = 0
                var fId: UInt32 = 0
                var rgba = [UInt8](repeating: 0, count: Int(XENORA_FIXTURE_FRAME_WIDTH * XENORA_FIXTURE_FRAME_HEIGHT * 4))
                let code = rgba.withUnsafeMutableBufferPointer { buffer in
                    xenora_session_frame_loop_status(
                        self.session, &st, &comp, &fId, buffer.baseAddress, UInt64(buffer.count))
                }
                guard code == XENORA_OK else { break }
                self.state = st
                self.completed = comp
                self.frameId = fId
                if fId != XENORA_NO_FRAME, let provider = CGDataProvider(data: Data(rgba) as CFData) {
                    self.image = CGImage(
                        width: Int(XENORA_FIXTURE_FRAME_WIDTH),
                        height: Int(XENORA_FIXTURE_FRAME_HEIGHT),
                        bitsPerComponent: 8,
                        bitsPerPixel: 32,
                        bytesPerRow: Int(XENORA_FIXTURE_FRAME_WIDTH) * 4,
                        space: CGColorSpaceCreateDeviceRGB(),
                        bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue),
                        provider: provider,
                        decode: nil,
                        shouldInterpolate: false,
                        intent: .defaultIntent
                    )
                }
                if st != UInt32(XENORA_LOOP_RUNNING) && st != UInt32(XENORA_LOOP_PAUSED) {
                    break
                }
            }
        }
    }
}

/// Read-only compatibility inspection of a user-chosen file. Nothing in it is executed.
@MainActor
final class InspectionModel: ObservableObject {
    static let shared = InspectionModel()
    @Published var fileName: String?
    @Published var report = ""
    @Published var status: Int32 = Int32(XENORA_OK)
    @Published var busy = false
    @Published var importing = false

    func inspect(_ url: URL) {
        fileName = url.lastPathComponent
        busy = true
        Task {
            let (newStatus, newReport) = await Task.detached { InspectionModel.run(url) }.value
            status = newStatus
            report = newReport
            busy = false
        }
    }

    nonisolated static func run(_ url: URL) -> (Int32, String) {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        // Check the size before reading so an oversized file is never loaded into memory.
        guard let size = (try? url.resourceValues(forKeys: [.fileSizeKey]))?.fileSize else {
            return (Int32(XENORA_INVALID_ARGUMENT), "Could not read the file's size.")
        }
        guard UInt64(size) <= UInt64(XENORA_MAX_INSPECT_BYTES) else {
            return (Int32(XENORA_LIMIT_EXCEEDED), "The file is \(size) bytes; inspection accepts at most \(XENORA_MAX_INSPECT_BYTES).")
        }
        guard let data = try? Data(contentsOf: url) else {
            return (Int32(XENORA_INVALID_ARGUMENT), "Could not read the file.")
        }
        return data.withUnsafeBytes { raw in
            let bytes = raw.bindMemory(to: UInt8.self).baseAddress
            var required: UInt64 = 0
            var probe: CChar = 0
            var status = xenora_inspect_executable(bytes, UInt64(data.count), &probe, 1, &required)
            guard status == Int32(XENORA_BUFFER_TOO_SMALL) else { return (status, "Inspection failed (status \(status)).") }
            var text = [CChar](repeating: 0, count: Int(required))
            status = xenora_inspect_executable(bytes, UInt64(data.count), &text, required, &required)
            return (status, String(cString: text))
        }
    }
}

/// Receives files opened from Finder (double-click or Open With).
final class AppDelegate: NSObject, NSApplicationDelegate {
    func application(_ application: NSApplication, open urls: [URL]) {
        guard let url = urls.first else { return }
        MainActor.assumeIsolated { InspectionModel.shared.inspect(url) }
    }
}

struct InspectorView: View {
    @ObservedObject var model: InspectionModel

    var body: some View {
        Section("Executable inspection (read-only)") {
            HStack {
                Button("Open Executable…") { model.importing = true }
                Text("or drop a file on this window").foregroundStyle(.secondary)
            }
            if let name = model.fileName {
                LabeledContent("File", value: model.busy ? "\(name) (inspecting…)" : name)
                ScrollView {
                    Text(model.report)
                        .font(.system(.caption, design: .monospaced))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(minHeight: 160, maxHeight: 320)
            }
            Text("Opened files are inspected only. Running them is not available.")
                .font(.callout)
                .foregroundStyle(.secondary)
        }
    }
}

struct ContentView: View {
    let probe = RuntimeProbe.run()
    @ObservedObject var model: FrameModel
    @ObservedObject var loop: FrameLoopModel
    @ObservedObject var inspection = InspectionModel.shared
    private var frame: GuestFrame? { model.frame }
    private var running: Bool { model.running }

    var body: some View {
        Form {
            InspectorView(model: inspection)
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
                if let tImage = model.texturedFrame?.image {
                    Image(decorative: tImage, scale: 1)
                        .resizable()
                        .interpolation(.none)
                        .aspectRatio(contentMode: .fit)
                        .frame(maxWidth: .infinity, minHeight: 80, maxHeight: 160)
                        .accessibilityLabel("Textured guest frame, 2 by 2 pixels")
                    Text("Textured fixture ran: persistent texture created, dynamically updated, rendered on Metal, and retired.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                if let aImage = model.audioVfsFrame?.image {
                    Image(decorative: aImage, scale: 1)
                        .resizable()
                        .interpolation(.none)
                        .aspectRatio(contentMode: .fit)
                        .frame(maxWidth: .infinity, minHeight: 80, maxHeight: 160)
                        .accessibilityLabel("Audio/VFS guest frame, 2 by 2 pixels")
                    Text("Audio/VFS fixture ran: asset.bin loaded through sandboxed VFS, \(model.audioVfsFrame!.samples) audio samples queued (\(model.audioVfsFrame!.totalAudioBytes) bytes), texture rendered on Metal.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                HStack {
                    Button(running ? "Running…" : "Run trusted fixture") { model.run() }
                        .disabled(running || !probe.connected)
                    Button(model.runningTextured ? "Running…" : "Run textured fixture") { model.runTextured() }
                        .disabled(model.runningTextured || !probe.connected)
                    Button(model.runningAudioVfs ? "Running…" : "Run Audio/VFS fixture") { model.runAudioVfs() }
                        .disabled(model.runningAudioVfs || !probe.connected)
                }
            }
            Section("Persistent frame loop") {
                if let image = loop.image {
                    Image(decorative: image, scale: 1)
                        .resizable()
                        .interpolation(.none)
                        .aspectRatio(contentMode: .fit)
                        .frame(maxWidth: .infinity, minHeight: 120, maxHeight: 240)
                        .accessibilityLabel("Guest frame #\(loop.frameId)")
                    Text("Frame #\(loop.frameId) (\(loop.completed) completed) • \(loop.stateDescription)")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                } else if let error = loop.errorMessage {
                    Text(error).foregroundStyle(.red)
                } else if loop.state != UInt32(XENORA_LOOP_NONE) {
                    Text("State: \(loop.stateDescription) • \(loop.completed) completed")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                HStack {
                    Button("Start 300 frames") { loop.start(frameCount: 300) }
                        .disabled(!loop.canStart || !probe.connected)
                    Button("Interactive (300 frames)") { loop.startInteractive(frameCount: 300) }
                        .disabled(!loop.canStart || !probe.connected)
                    Button("Pause") { loop.pause() }
                        .disabled(!loop.isRunning)
                    Button("Resume") { loop.resume() }
                        .disabled(!loop.isPaused)
                    Button("Stop") { loop.stop() }
                        .disabled(!loop.isRunning && !loop.isPaused)
                }
                if loop.isInteractive && (loop.isRunning || loop.isPaused) {
                    HStack {
                        Button("◀ Left") { loop.sendInput(buttons: UInt32(XENORA_INPUT_LEFT)) }
                        Button("▶ Right") { loop.sendInput(buttons: UInt32(XENORA_INPUT_RIGHT)) }
                        Button("Release") { loop.sendInput(buttons: UInt32(XENORA_INPUT_NONE)) }
                    }
                    Text("Interactive guest: steer the rendered strip across columns 0..3 with Left/Right.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            LabeledContent("Runtime", value: probe.connected ? "Connected" : "Unavailable (status \(probe.status))")
            LabeledContent("Interface version", value: "\(probe.abiVersion)")
            LabeledContent("Trusted fixture sections", value: "\(probe.sections)")
            LabeledContent("Trusted fixture entry RVA", value: String(format: "0x%X", probe.entryRVA))
            Text("Only the bundled trusted fixtures (single-frame, frame-loop, and interactive guest) can run. Presentation uses read-back Metal frames. Arbitrary executables are not available yet.")
                .foregroundStyle(.secondary)
        }
        .formStyle(.grouped)
        .fileImporter(isPresented: $inspection.importing, allowedContentTypes: [.item]) { result in
            if case .success(let url) = result { inspection.inspect(url) }
        }
        .dropDestination(for: URL.self) { urls, _ in
            guard let url = urls.first, url.isFileURL else { return false }
            inspection.inspect(url)
            return true
        }
        .frame(minWidth: 520, minHeight: 680)
    }
}

struct XenoraApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) var delegate
    let model = FrameModel()
    let loop = FrameLoopModel()

    var body: some Scene {
        WindowGroup("Xenora") { ContentView(model: model, loop: loop) }
    }
}

// `--inspect <path>` prints the read-only report for a file and exits with its status.
if let flag = CommandLine.arguments.firstIndex(of: "--inspect"), flag + 1 < CommandLine.arguments.count {
    let (status, report) = InspectionModel.run(URL(fileURLWithPath: CommandLine.arguments[flag + 1]))
    print(report.hasSuffix("\n") ? report : report + "\n", terminator: "")
    print("status=\(status)")
    exit(status == Int32(XENORA_OK) ? 0 : 1)
}

// `--smoke-resources` tests persistent texture lifecycle, dynamic subregion updates, and retirement on Metal.
if CommandLine.arguments.contains("--smoke-resources") {
    let frame = TexturedFrame.run()
    print("smoke-resources: \(frame.summary)")
    let expected: [UInt8] = [255, 255, 0, 255, 0, 255, 255, 255, 255, 0, 255, 255, 64, 64, 64, 255]
    exit(frame.status == XENORA_OK && frame.width == 2 && frame.height == 2 && frame.rgba == expected ? 0 : 1)
}

// `--smoke-audio-vfs` tests VFS asset loading, PCM audio streaming, and Metal texture presentation.
if CommandLine.arguments.contains("--smoke-native-audio") {
    let frame = AudioVfsFrame.run(captureAudio: true)
    guard frame.status == XENORA_OK, frame.samples == 256, frame.pcm.count == 512 else {
        print("smoke-native-audio: fixture PCM unavailable")
        exit(1)
    }
    do {
        let played = try playFixturePCM(frame.pcm)
        print("smoke-native-audio: playedBackFrames=\(played)")
        exit(played == Int(frame.samples) ? 0 : 1)
    } catch {
        print("smoke-native-audio: \(error)")
        exit(1)
    }
}

if CommandLine.arguments.contains("--smoke-audio-vfs") {
    let frame = AudioVfsFrame.run()
    print("smoke-audio-vfs: \(frame.summary)")
    let expected: [UInt8] = [255, 128, 0, 255, 128, 0, 255, 255, 0, 255, 128, 255, 255, 0, 128, 255]
    exit(frame.status == XENORA_OK && frame.width == 2 && frame.height == 2 && frame.samples == 256 && frame.rgba == expected && frame.totalAudioBytes == 512 ? 0 : 1)
}

// `--smoke-interactive` runs 60 frames with queued inputs on the Metal device and exits.
if CommandLine.arguments.contains("--smoke-interactive") {
    var session: xenora_session = 0
    guard xenora_session_create(XENORA_ABI_VERSION, &session) == XENORA_OK else {
        print("failed to create session")
        exit(1)
    }
    defer { _ = xenora_session_destroy(session) }
    guard xenora_session_start_interactive_loop(session, 60) == XENORA_OK else {
        print("failed to start interactive loop")
        exit(1)
    }
    for f in 0..<60 {
        let btn: UInt32 = (f < 15) ? UInt32(XENORA_INPUT_RIGHT) :
                          (f < 30) ? UInt32(XENORA_INPUT_NONE) :
                          (f < 45) ? UInt32(XENORA_INPUT_LEFT) : UInt32(XENORA_INPUT_RIGHT)
        var input = xenora_input_state(buttons: btn, stick_x: 0, stick_y: 0, mouse_dx: 0, mouse_dy: 0, mouse_buttons: 0, connected: 1, reserved: 0)
        _ = xenora_session_queue_input(session, &input)
    }
    var state: UInt32 = 0
    var completed: UInt32 = 0
    var frameId: UInt32 = 0
    var rgba = [UInt8](repeating: 0, count: Int(XENORA_FIXTURE_FRAME_WIDTH * XENORA_FIXTURE_FRAME_HEIGHT * 4))
    while true {
        let code = rgba.withUnsafeMutableBufferPointer { buf in
            xenora_session_frame_loop_status(session, &state, &completed, &frameId, buf.baseAddress, UInt64(buf.count))
        }
        if code != XENORA_OK {
            print("frame_loop_status failed: \(code)")
            exit(1)
        }
        if state != UInt32(XENORA_LOOP_RUNNING) { break }
        Thread.sleep(forTimeInterval: 0.005)
    }
    print("interactive: state=\(state) completed=\(completed) frame_id=\(frameId)")
    exit(state == UInt32(XENORA_LOOP_FINISHED) && completed == 60 ? 0 : 1)
}

// `--smoke-loop` runs 300 frames on the Metal device and exits; it needs a GPU.
if CommandLine.arguments.contains("--smoke-loop") {
    var session: xenora_session = 0
    guard xenora_session_create(XENORA_ABI_VERSION, &session) == XENORA_OK else {
        print("failed to create session")
        exit(1)
    }
    defer { _ = xenora_session_destroy(session) }
    guard xenora_session_start_frame_loop(session, 300) == XENORA_OK else {
        print("failed to start frame loop")
        exit(1)
    }
    var state: UInt32 = 0
    var completed: UInt32 = 0
    var frameId: UInt32 = 0
    var rgba = [UInt8](repeating: 0, count: Int(XENORA_FIXTURE_FRAME_WIDTH * XENORA_FIXTURE_FRAME_HEIGHT * 4))
    while true {
        let code = rgba.withUnsafeMutableBufferPointer { buf in
            xenora_session_frame_loop_status(session, &state, &completed, &frameId, buf.baseAddress, UInt64(buf.count))
        }
        if code != XENORA_OK {
            print("frame_loop_status failed: \(code)")
            exit(1)
        }
        if state != UInt32(XENORA_LOOP_RUNNING) { break }
        Thread.sleep(forTimeInterval: 0.005)
    }
    print("state=\(state) completed=\(completed) frame_id=\(frameId)")
    exit(state == UInt32(XENORA_LOOP_FINISHED) && completed == 300 ? 0 : 1)
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
