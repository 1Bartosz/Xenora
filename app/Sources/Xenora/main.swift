// SPDX-License-Identifier: GPL-3.0-or-later
//
// Gate 1 shell: proves the app links the Rust runtime through the versioned C
// interface. It inspects the bundled trusted fixture and executes no guest code.

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

struct ContentView: View {
    let probe = RuntimeProbe.run()

    var body: some View {
        Form {
            LabeledContent("Runtime", value: probe.connected ? "Connected" : "Unavailable (status \(probe.status))")
            LabeledContent("Interface version", value: "\(probe.abiVersion)")
            LabeledContent("Trusted fixture sections", value: "\(probe.sections)")
            LabeledContent("Trusted fixture entry RVA", value: String(format: "0x%X", probe.entryRVA))
            Text("Guest execution and live display are not available in this build.")
                .foregroundStyle(.secondary)
        }
        .formStyle(.grouped)
        .frame(minWidth: 420, minHeight: 240)
    }
}

struct XenoraApp: App {
    var body: some Scene {
        WindowGroup("Xenora") { ContentView() }
    }
}

// `--smoke` prints the probe and exits without opening a window, for scripts and CI.
if CommandLine.arguments.contains("--smoke") {
    let probe = RuntimeProbe.run()
    print(probe.summary)
    exit(probe.connected ? 0 : 1)
}
XenoraApp.main()
