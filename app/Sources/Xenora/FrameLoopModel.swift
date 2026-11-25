// SPDX-License-Identifier: GPL-3.0-or-later
import CoreGraphics
import CXenora
import Foundation
import SwiftUI

enum FrameLoopMode {
    case standard
    case interactive

    var startOperation: String {
        switch self {
        case .standard: "start frame loop"
        case .interactive: "start interactive loop"
        }
    }
}

protocol FrameLoopAPI: Sendable {
    func create(_ handle: inout xenora_session) -> Int32
    func destroy(_ handle: xenora_session) -> Int32
    func start(_ handle: xenora_session, mode: FrameLoopMode, frames: UInt32) -> Int32
    func sendInput(_ handle: xenora_session, _ input: inout xenora_controller_input) -> Int32
    func pause(_ handle: xenora_session) -> Int32
    func resume(_ handle: xenora_session) -> Int32
    func stop(_ handle: xenora_session) -> Int32
    func status(
        _ handle: xenora_session, _ state: inout UInt32, _ completed: inout UInt32,
        _ frame: inout UInt32, _ rgba: UnsafeMutableBufferPointer<UInt8>
    ) -> Int32
}

struct NativeFrameLoopAPI: FrameLoopAPI {
    func create(_ handle: inout xenora_session) -> Int32 {
        xenora_session_create(XENORA_ABI_VERSION, &handle)
    }

    func destroy(_ handle: xenora_session) -> Int32 {
        xenora_session_destroy(handle)
    }

    func start(_ handle: xenora_session, mode: FrameLoopMode, frames: UInt32) -> Int32 {
        switch mode {
        case .standard: xenora_session_start_frame_loop(handle, frames)
        case .interactive: xenora_session_start_interactive_loop(handle, frames)
        }
    }

    func sendInput(_ handle: xenora_session, _ input: inout xenora_controller_input) -> Int32 {
        xenora_session_send_controller_input(handle, &input)
    }

    func pause(_ handle: xenora_session) -> Int32 { xenora_session_pause_frame_loop(handle) }
    func resume(_ handle: xenora_session) -> Int32 { xenora_session_resume_frame_loop(handle) }
    func stop(_ handle: xenora_session) -> Int32 { xenora_session_stop_frame_loop(handle) }

    func status(
        _ handle: xenora_session, _ state: inout UInt32, _ completed: inout UInt32,
        _ frame: inout UInt32, _ rgba: UnsafeMutableBufferPointer<UInt8>
    ) -> Int32 {
        xenora_session_frame_loop_status(handle, &state, &completed, &frame, rgba.baseAddress, UInt64(rgba.count))
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
    /// Completed frames per second over the last half second, measured on the host.
    @Published var hostFps: Double?
    @Published private(set) var controller: DetectedController?

    private let api: any FrameLoopAPI
    private let readController: @MainActor () -> DetectedController?
    private var session: xenora_session = 0
    private var pollTask: Task<Void, Never>?

    init(
        api: any FrameLoopAPI = NativeFrameLoopAPI(),
        readController: @escaping @MainActor () -> DetectedController? = pollGameController
    ) {
        self.api = api
        self.readController = readController
    }

    deinit {
        pollTask?.cancel()
        if session != 0 {
            _ = api.destroy(session)
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
        start(.standard, frameCount: frameCount)
    }

    func startInteractive(frameCount: UInt32 = 300) {
        start(.interactive, frameCount: frameCount)
    }

    private func start(_ mode: FrameLoopMode, frameCount: UInt32) {
        errorMessage = nil
        cancelPolling()
        state = UInt32(XENORA_LOOP_FAILED)
        isInteractive = false
        controller = nil
        completed = 0
        frameId = XENORA_NO_FRAME
        image = nil
        hostFps = nil

        guard destroySession() else { return }
        var newSession: xenora_session = 0
        let created = api.create(&newSession)
        guard check(created, operation: "create fixture session") else { return }

        session = newSession
        guard check(api.start(session, mode: mode, frames: frameCount), operation: mode.startOperation) else {
            _ = destroySession()
            return
        }
        isInteractive = mode == .interactive
        state = UInt32(XENORA_LOOP_RUNNING)
        startPolling(session)
    }

    private func cancelPolling() {
        pollTask?.cancel()
        pollTask = nil
    }

    // Keep the handle for another cleanup attempt if destruction failed and
    // the native session might still exist.
    private func destroySession() -> Bool {
        guard session != 0 else { return true }
        let code = api.destroy(session)
        if code == XENORA_OK || code == XENORA_INVALID_HANDLE {
            session = 0
        }
        return check(code, operation: "destroy fixture session", append: true)
    }

    @discardableResult
    private func check(_ code: Int32, operation: String, append: Bool = false) -> Bool {
        guard code != XENORA_OK else { return true }
        let message = "Failed to \(operation) (status \(code))."
        if append, let previous = errorMessage {
            errorMessage = previous + " " + message
        } else {
            errorMessage = message
        }
        return false
    }

    func sendInput(buttons: UInt32) {
        sendInput(buttons: buttons, x: 0, y: 0, connected: true)
    }

    private func sendInput(
        buttons: UInt32, x: Int16, y: Int16, connected: Bool,
        leftTrigger: Float = 0, rightTrigger: Float = 0
    ) {
        guard session != 0, isInteractive else { return }
        let state = xenora_input_state(
            buttons: buttons,
            stick_x: x,
            stick_y: y,
            mouse_dx: 0,
            mouse_dy: 0,
            mouse_buttons: 0,
            connected: connected ? 1 : 0,
            reserved: 0
        )
        var input = xenora_controller_input(
            state: state, left_trigger: UInt16((leftTrigger * 65535).rounded()),
            right_trigger: UInt16((rightTrigger * 65535).rounded())
        )
        check(api.sendInput(session, &input), operation: "send controller input")
    }

    private func updateGameController() {
        let next = readController()
        guard next != controller else { return }
        controller = next
        let snapshot = next?.snapshot
        sendInput(buttons: snapshot?.buttons ?? 0, x: snapshot?.x ?? 0, y: snapshot?.y ?? 0,
                  connected: next != nil, leftTrigger: snapshot?.leftTrigger ?? 0,
                  rightTrigger: snapshot?.rightTrigger ?? 0)
    }

    func pause() {
        guard session != 0 else { return }
        check(api.pause(session), operation: "pause frame loop")
    }

    func resume() {
        guard session != 0 else { return }
        check(api.resume(session), operation: "resume frame loop")
    }

    func stop() {
        guard session != 0 else { return }
        check(api.stop(session), operation: "stop frame loop")
    }

    private func startPolling(_ handle: xenora_session) {
        pollTask = Task { [weak self] in
            var windowStart = Date()
            var windowFrames: UInt32? = nil
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 16_000_000)
                // Cancellation can arrive while this task is asleep. Check it
                // before using either the old handle or the replacement's state.
                guard !Task.isCancelled, let self, self.session == handle else { return }
                var st: UInt32 = 0
                var comp: UInt32 = 0
                var fId: UInt32 = XENORA_NO_FRAME
                var rgba = [UInt8](repeating: 0, count: Int(XENORA_FIXTURE_FRAME_WIDTH * XENORA_FIXTURE_FRAME_HEIGHT * 4))
                let code = rgba.withUnsafeMutableBufferPointer { buffer in
                    self.api.status(handle, &st, &comp, &fId, buffer)
                }
                guard self.check(code, operation: "read frame loop status") else {
                    self.state = UInt32(XENORA_LOOP_FAILED)
                    self.cancelPolling()
                    _ = self.destroySession()
                    return
                }
                if self.isInteractive && (st == XENORA_LOOP_RUNNING || st == XENORA_LOOP_PAUSED) {
                    self.updateGameController()
                }
                self.state = st
                self.completed = comp
                self.frameId = fId
                let elapsed = Date().timeIntervalSince(windowStart)
                if let first = windowFrames {
                    if elapsed >= 0.5 {
                        self.hostFps = Double(comp &- first) / elapsed
                        windowStart = Date()
                        windowFrames = comp
                    }
                } else {
                    windowFrames = comp
                    windowStart = Date()
                }
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
                    // A loop that ends inside its first window still gets a rate.
                    if let first = windowFrames, comp > first, elapsed > 0 {
                        self.hostFps = Double(comp - first) / elapsed
                    }
                    if st == XENORA_LOOP_FAILED && self.errorMessage == nil {
                        self.errorMessage = "Fixture frame loop entered the failed state."
                    }
                    self.cancelPolling()
                    return
                }
            }
        }
    }
}
