/*
 *  UltraVoxMacOSBridge.swift
 *  Minimal macOS-native bridge callable from Rust.
 *
 *  This file exposes @_cdecl functions for Rust/Tauri UltraVox parity.
 *  The native baseline is the UltraVox Swift codebase; these
 *  implementations are self-contained in this static library and do not
 *  modify the original app.
 *
 *  License: MIT (see /LICENSE in the repository root)
 */

import AppKit
@preconcurrency import ApplicationServices
import AVFoundation
import Carbon
import Cocoa
import Darwin
import Foundation

@preconcurrency import ScreenCaptureKit

#if canImport(FluidAudio)
import FluidAudio
#endif

// MARK: - FluidAudio / CoreML transcription engine

#if canImport(FluidAudio)
private enum FluidAudioTranscriptionError: Error {
    case engineUnavailable
    case noResult
}

private final class ModelProgressStore: @unchecked Sendable {
    static let shared = ModelProgressStore()
    private let lock = NSLock()
    private var values: [String: Double] = [:]

    func set(_ value: Double, for version: String) {
        lock.lock()
        values[version.lowercased()] = value
        lock.unlock()
    }

    func value(for version: String) -> Double {
        lock.lock()
        defer { lock.unlock() }
        return values[version.lowercased()] ?? 0
    }
}

private actor FluidAudioTranscriptionEngine {
    static let shared = FluidAudioTranscriptionEngine()

    private var asrManager: AsrManager?
    private var asrModels: AsrModels?
    private var loadedVersion: AsrModelVersion?
    private var loadedDirectory: URL?

    private var activeTask: Task<String, Error>?
    private var activeRecordingId: String?
    private var cancelledRecordingIds = Set<String>()

    private func version(for string: String) -> AsrModelVersion {
        string.lowercased() == "v3" ? .v3 : .v2
    }

    private func config(for version: AsrModelVersion) -> ASRConfig {
        // v3 multilingual benefits from the no-mel-context path; v2 English
        // uses the default configuration that preserves the boundary-fix warmup.
        version == .v3
            ? ASRConfig(melChunkContext: false, dualDecodeArbitration: true)
            : ASRConfig()
    }

    func ensureLoaded(versionString: String, directory: URL? = nil) async throws {
        let version = version(for: versionString)
        let normalizedDirectory = directory?.standardizedFileURL
        guard loadedVersion != version || loadedDirectory != normalizedDirectory || asrManager == nil || asrModels == nil else {
            return
        }

        ModelProgressStore.shared.set(0, for: versionString)
        let models = try await AsrModels.downloadAndLoad(
            to: directory,
            version: version,
            progressHandler: { progress in
                ModelProgressStore.shared.set(progress.fractionCompleted, for: versionString)
            }
        )
        let manager = AsrManager(config: config(for: version))
        try await manager.loadModels(models)

        asrModels = models
        asrManager = manager
        loadedVersion = version
        loadedDirectory = normalizedDirectory
        ModelProgressStore.shared.set(1, for: versionString)
    }

    /// Cancels or pre-cancels the transcription for the given recording identity.
    /// Recording the identity before checking the active task closes the handoff
    /// race where Rust has reserved a job but this actor has not claimed it yet.
    /// The pending cancellation is removed by `transcribe` when the job arrives.
    func cancelTranscription(recordingId: String) -> Bool {
        cancelledRecordingIds.insert(recordingId)
        if activeRecordingId == recordingId {
            activeTask?.cancel()
        }
        return true
    }

    func transcribe(
        url: URL,
        versionString: String,
        recordingId: String,
        directory: URL? = nil
    ) async throws -> String {
        // If this recording was already cancelled before reaching the actor, stop
        // immediately without loading models or consuming the shared engine.
        if cancelledRecordingIds.contains(recordingId) {
            cancelledRecordingIds.remove(recordingId)
            throw CancellationError()
        }

        // Claim the shared engine before model loading so cancellation can be
        // recorded while a download/load is in progress. Never replace an
        // existing owner: an older call must retain its cancellation identity.
        guard activeRecordingId == nil, activeTask == nil else {
            throw CancellationError()
        }
        activeRecordingId = recordingId

        defer {
            if activeRecordingId == recordingId {
                activeTask = nil
                activeRecordingId = nil
            }
            cancelledRecordingIds.remove(recordingId)
        }

        try await ensureLoaded(versionString: versionString, directory: directory)

        // Re-check cancellation after model readiness and before transcription.
        if cancelledRecordingIds.contains(recordingId) {
            throw CancellationError()
        }
        guard let manager = asrManager else {
            throw FluidAudioTranscriptionError.engineUnavailable
        }

        let task = Task { [weak manager] () -> String in
            try Task.checkCancellation()
            guard let manager = manager else {
                throw FluidAudioTranscriptionError.engineUnavailable
            }
            var decoderState = TdtDecoderState.make(decoderLayers: await manager.decoderLayerCount)
            try Task.checkCancellation()
            let result = try await manager.transcribe(url, decoderState: &decoderState)
            try Task.checkCancellation()
            return result.text.trimmingCharacters(in: .whitespacesAndNewlines)
        }

        activeTask = task
        do {
            let text = try await task.value
            return text
        } catch is CancellationError {
            throw CancellationError()
        }
    }
}

/// Convert a version string to a FluidAudio model version.
private func asrVersion(from string: String) -> AsrModelVersion {
    string.lowercased() == "v3" ? .v3 : .v2
}

/// Runs an async closure from a synchronous C-callable entry point and returns
/// its result, blocking the calling thread until the work completes.
private func runAsyncAndBlock<T: Sendable>(
    _ operation: @Sendable @escaping () async throws -> T
) throws -> T {
    let box = AsyncResultBox<T>()

    Task {
        do {
            box.setResult(try await operation())
        } catch {
            box.setError(error)
        }
    }

    return try box.wait()
}

private final class AsyncResultBox<T: Sendable>: @unchecked Sendable {
    private let lock = NSLock()
    private let semaphore = DispatchSemaphore(value: 0)
    private var result: T?
    private var error: Error?

    func setResult(_ result: T) {
        lock.lock()
        self.result = result
        lock.unlock()
        semaphore.signal()
    }

    func setError(_ error: Error) {
        lock.lock()
        self.error = error
        lock.unlock()
        semaphore.signal()
    }

    func wait() throws -> T {
        semaphore.wait()
        lock.lock()
        defer { lock.unlock() }
        if let error = error {
            throw error
        }
        guard let result = result else {
            throw FluidAudioTranscriptionError.noResult
        }
        return result
    }
}

#endif

/// Runs a `@MainActor` closure synchronously, avoiding deadlock if the caller is already on the main thread.
private func runOnMainActor<T: Sendable>(body: @MainActor @escaping () -> T) -> T {
    if Thread.isMainThread {
        return MainActor.assumeIsolated(body)
    }
    let box = ResultBox<T>()
    let semaphore = DispatchSemaphore(value: 0)
    Task { @MainActor in
        box.set(body())
        semaphore.signal()
    }
    semaphore.wait()
    return box.value
}

private final class ResultBox<T: Sendable>: @unchecked Sendable {
    private let lock = NSLock()
    private var storedValue: T?

    func set(_ value: T) {
        lock.lock()
        storedValue = value
        lock.unlock()
    }

    var value: T {
        lock.lock()
        defer { lock.unlock() }
        return storedValue!
    }
}

@available(macOS 15.0, *)
private enum MeetingCaptureError: LocalizedError {
    case noDisplay
    case alreadyRecording
    case notRecording
    case recordingDidNotFinish
    case exportUnavailable
    case exportFailed(String)

    var errorDescription: String? {
        switch self {
        case .noDisplay:
            return "UltraVox could not find a display to capture."
        case .alreadyRecording:
            return "Meeting mode is already recording."
        case .notRecording:
            return "Meeting mode is not recording."
        case .recordingDidNotFinish:
            return "The meeting recording did not finish writing."
        case .exportUnavailable:
            return "UltraVox could not prepare the meeting audio track."
        case let .exportFailed(details):
            return "UltraVox could not extract meeting audio: \(details)"
        }
    }
}

@available(macOS 15.0, *)
private final class MeetingCaptureManager: NSObject, SCRecordingOutputDelegate, @unchecked Sendable {
    static let shared = MeetingCaptureManager()

    private let lock = NSLock()
    private var stream: SCStream?
    private var recordingOutput: SCRecordingOutput?
    private var videoURL: URL?
    private var finishSignal: DispatchSemaphore?
    private var finishError: Error?
    private var recordingStarted = false
    private var recordingFinished = false
    private var startContinuation: CheckedContinuation<Void, Error>?
    private var failureCallback: (@convention(c) (UnsafePointer<CChar>?) -> Void)?

    func setFailureCallback(
        _ callback: (@convention(c) (UnsafePointer<CChar>?) -> Void)?
    ) {
        lock.withLock {
            failureCallback = callback
        }
    }

    private func notifyFailure(_ error: Error) {
        let callback = lock.withLock { failureCallback }
        guard let callback else { return }
        let cString = error.localizedDescription.duplicateAsCChar()
        callback(cString)
        free(cString)
    }

    private func waitForRecordingStart() async throws {
        try await withCheckedThrowingContinuation {
            (continuation: CheckedContinuation<Void, Error>) in
            lock.lock()
            if let finishError {
                lock.unlock()
                continuation.resume(throwing: finishError)
            } else if recordingStarted {
                lock.unlock()
                continuation.resume()
            } else {
                startContinuation = continuation
                lock.unlock()
            }
        }
    }

    func start(outputURL: URL, captureMicrophone: Bool) async throws {
        let alreadyRecording = lock.withLock { self.stream != nil }
        if alreadyRecording {
            throw MeetingCaptureError.alreadyRecording
        }

        let content = try await SCShareableContent.excludingDesktopWindows(
            false,
            onScreenWindowsOnly: true
        )
        guard let display = content.displays.first else {
            throw MeetingCaptureError.noDisplay
        }

        try? FileManager.default.removeItem(at: outputURL)
        let filter = SCContentFilter(display: display, excludingWindows: [])
        let streamConfiguration = SCStreamConfiguration()
        streamConfiguration.width = 16
        streamConfiguration.height = 16
        streamConfiguration.minimumFrameInterval = CMTime(value: 1, timescale: 1)
        streamConfiguration.queueDepth = 1
        streamConfiguration.showsCursor = false
        streamConfiguration.capturesAudio = true
        streamConfiguration.excludesCurrentProcessAudio = true
        streamConfiguration.captureMicrophone = captureMicrophone
        // SCRecordingOutput requires a video stream. Keep it deliberately tiny;
        // only the mixed audio track survives after stop().
        streamConfiguration.width = 16
        streamConfiguration.height = 16
        streamConfiguration.minimumFrameInterval = CMTime(value: 1, timescale: 1)
        streamConfiguration.queueDepth = 3

        let recordingConfiguration = SCRecordingOutputConfiguration()
        if recordingConfiguration.availableOutputFileTypes.contains(.mp4) {
            recordingConfiguration.outputFileType = .mp4
        } else if let firstType = recordingConfiguration.availableOutputFileTypes.first {
            recordingConfiguration.outputFileType = firstType
        }
        recordingConfiguration.outputURL = outputURL
        if recordingConfiguration.availableVideoCodecTypes.contains(.h264) {
            recordingConfiguration.videoCodecType = .h264
        }

        let recordingOutput = SCRecordingOutput(
            configuration: recordingConfiguration,
            delegate: self
        )
        let stream = SCStream(filter: filter, configuration: streamConfiguration, delegate: nil)
        try stream.addRecordingOutput(recordingOutput)

        lock.withLock {
            self.stream = stream
            self.recordingOutput = recordingOutput
            self.videoURL = outputURL
            finishError = nil
            recordingStarted = false
            recordingFinished = false
            startContinuation = nil
        }

        do {
            try await stream.startCapture()
            try await waitForRecordingStart()
        } catch {
            try? await stream.stopCapture()
            lock.withLock {
                self.stream = nil
                self.recordingOutput = nil
                self.videoURL = nil
                finishSignal = nil
                finishError = nil
                recordingStarted = false
                recordingFinished = false
                startContinuation = nil
            }
            try? FileManager.default.removeItem(at: outputURL)
            throw error
        }
    }

    func stop() async throws -> URL {
        let activeCapture = lock.withLock {
            () -> (SCStream, URL, DispatchSemaphore, Bool, Error?)? in
            guard let stream, let videoURL else { return nil }
            let signal = DispatchSemaphore(value: 0)
            finishSignal = signal
            return (stream, videoURL, signal, recordingFinished, finishError)
        }
        guard let (stream, videoURL, signal, alreadyFinished, existingRecordingError) = activeCapture else {
            throw MeetingCaptureError.notRecording
        }

        var stopError: Error?
        do {
            try await stream.stopCapture()
        } catch {
            stopError = error
        }
        // A successful stopCapture() is always answered by either
        // recordingOutputDidFinishRecording or didFailWithError, and both signal
        // this semaphore. Long recordings can take tens of seconds to finalize
        // after stopCapture returns, so wait without an arbitrary timeout. The
        // finish callback may also have fired before stop() was called; the
        // recordingFinished flag covers that case so a retry can still succeed.
        let didFinish: Bool
        if alreadyFinished || stopError != nil || existingRecordingError != nil {
            didFinish = alreadyFinished
        } else {
            await Task.detached {
                Self.waitForFinish(signal)
            }.value
            didFinish = true
        }

        let recordingError = lock.withLock { () -> Error? in
            defer {
                self.stream = nil
                recordingOutput = nil
                self.videoURL = nil
                finishSignal = nil
                finishError = nil
                recordingStarted = false
                recordingFinished = false
                startContinuation = nil
            }
            return finishError
        }

        if let stopError {
            throw stopError
        }
        if let recordingError {
            throw recordingError
        }
        guard didFinish else {
            throw MeetingCaptureError.recordingDidNotFinish
        }
        return try await exportAudio(from: videoURL)
    }

    private static func waitForFinish(_ signal: DispatchSemaphore) {
        signal.wait()
    }

    private func exportAudio(from videoURL: URL) async throws -> URL {
        let audioURL = videoURL.deletingPathExtension().appendingPathExtension("m4a")
        try? FileManager.default.removeItem(at: audioURL)
        let asset = AVURLAsset(url: videoURL)
        let sourceTracks = try await asset.loadTracks(withMediaType: .audio)
        guard !sourceTracks.isEmpty else {
            throw MeetingCaptureError.exportUnavailable
        }
        print("[UltraVoxMacOSBridge] exporting \(sourceTracks.count) captured audio track(s)")

        let composition = AVMutableComposition()
        var inputParameters: [AVMutableAudioMixInputParameters] = []
        let volume: Float = sourceTracks.count > 1 ? 0.7 : 1
        for sourceTrack in sourceTracks {
            guard let compositionTrack = composition.addMutableTrack(
                withMediaType: .audio,
                preferredTrackID: kCMPersistentTrackID_Invalid
            ) else {
                throw MeetingCaptureError.exportUnavailable
            }
            let sourceRange = try await sourceTrack.load(.timeRange)
            try compositionTrack.insertTimeRange(sourceRange, of: sourceTrack, at: .zero)
            let parameters = AVMutableAudioMixInputParameters(track: compositionTrack)
            parameters.setVolume(volume, at: .zero)
            inputParameters.append(parameters)
        }

        guard let exporter = AVAssetExportSession(
            asset: composition,
            presetName: AVAssetExportPresetAppleM4A
        ) else {
            throw MeetingCaptureError.exportUnavailable
        }
        let audioMix = AVMutableAudioMix()
        audioMix.inputParameters = inputParameters
        exporter.audioMix = audioMix
        exporter.outputURL = audioURL
        exporter.outputFileType = .m4a
        await exporter.export()
        guard exporter.status == .completed else {
            throw MeetingCaptureError.exportFailed(
                exporter.error?.localizedDescription ?? "unknown export error"
            )
        }
        try? FileManager.default.removeItem(at: videoURL)
        return audioURL
    }

    func recordingOutputDidStartRecording(_ recordingOutput: SCRecordingOutput) {
        lock.lock()
        recordingStarted = true
        let continuation = startContinuation
        startContinuation = nil
        lock.unlock()
        continuation?.resume()
    }

    func recordingOutputDidFinishRecording(_ recordingOutput: SCRecordingOutput) {
        lock.lock()
        recordingFinished = true
        let signal = finishSignal
        lock.unlock()
        signal?.signal()
    }

    func recordingOutput(
        _ recordingOutput: SCRecordingOutput,
        didFailWithError error: any Error
    ) {
        lock.lock()
        finishError = error
        let continuation = startContinuation
        startContinuation = nil
        let signal = finishSignal
        lock.unlock()
        continuation?.resume(throwing: error)
        signal?.signal()
        notifyFailure(error)
    }
}

// MARK: - Public C ABI

/// Returns the bridge version string.
@_cdecl("ultravox_macos_bridge_version")
public func ultravox_macos_bridge_version() -> UnsafeMutablePointer<CChar> {
    "UltraVoxMacOSBridge 0.5.0".duplicateAsCChar()
}

/// Frees a string previously returned by this bridge.
@_cdecl("ultravox_macos_bridge_free_string")
public func ultravox_macos_bridge_free_string(_ s: UnsafeMutablePointer<CChar>?) {
    if let s = s {
        free(s)
    }
}

// MARK: - Microphone authorization

/// Returns 0 for not determined, 1 for authorized, 2 for denied, and 3 for restricted.
@_cdecl("ultravox_macos_bridge_microphone_authorization_status")
public func ultravox_macos_bridge_microphone_authorization_status() -> Int32 {
    microphoneAuthorizationCode(AVCaptureDevice.authorizationStatus(for: .audio))
}

func microphoneAuthorizationCode(_ status: AVAuthorizationStatus) -> Int32 {
    switch status {
    case .notDetermined:
        return 0
    case .authorized:
        return 1
    case .denied:
        return 2
    case .restricted:
        return 3
    @unknown default:
        return 3
    }
}

/// Preserve the legacy boolean ABI; new callers use the detailed outcome API.
@_cdecl("ultravox_macos_bridge_request_microphone_access")
public func ultravox_macos_bridge_request_microphone_access() -> Int32 {
    ultravox_macos_bridge_request_microphone_access_outcome() == 1 ? 1 : 0
}

/// A timeout or concurrent request is unresolved, never evidence of denial.
func microphoneRequestOutcome(status: AVAuthorizationStatus, timedOut: Bool) -> Int32 {
    let code = microphoneAuthorizationCode(status)
    return code == 0 && timedOut ? 4 : code
}

final class MicrophoneRequestGate: @unchecked Sendable {
    private let lock = NSLock()
    private var requested = false

    func claim() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard !requested else { return false }
        requested = true
        return true
    }
}

private let microphoneRequestGate = MicrophoneRequestGate()

/// Explicit UI action only: 0 pending, 1 authorized, 2 denied, 3 restricted, 4 timeout.
/// Keep the request claimed even after timeout so rechecks cannot repeat OS consent.
@_cdecl("ultravox_macos_bridge_request_microphone_access_outcome")
public func ultravox_macos_bridge_request_microphone_access_outcome() -> Int32 {
    let status = AVCaptureDevice.authorizationStatus(for: .audio)
    guard status == .notDetermined else { return microphoneAuthorizationCode(status) }
    guard microphoneRequestGate.claim() else {
        return microphoneAuthorizationCode(AVCaptureDevice.authorizationStatus(for: .audio))
    }
    let semaphore = DispatchSemaphore(value: 0)
    AVCaptureDevice.requestAccess(for: .audio) { _ in semaphore.signal() }
    let timedOut = semaphore.wait(timeout: .now() + 60) != .success
    // Native status, not the callback boolean, remains the source of truth.
    return microphoneRequestOutcome(
        status: AVCaptureDevice.authorizationStatus(for: .audio), timedOut: timedOut
    )
}

/// Main-process bundle metadata only; no settings, content, or inferred helper identity.
@_cdecl("ultravox_macos_bridge_main_bundle_identity")
public func ultravox_macos_bridge_main_bundle_identity() -> UnsafeMutablePointer<CChar>? {
    let bundle = Bundle.main
    let identity: [String: Any] = [
        "bundleId": bundle.bundleIdentifier.map { $0 as Any } ?? NSNull(),
        "bundlePath": bundle.bundleURL.path,
        "executablePath": bundle.executableURL.map { $0.path as Any } ?? NSNull(),
    ]
    guard let data = try? JSONSerialization.data(withJSONObject: identity),
          let json = String(data: data, encoding: .utf8) else { return nil }
    return json.duplicateAsCChar()
}

/// Uses Apple's runtime preflight rather than stale TCC database rows.
@_cdecl("ultravox_macos_bridge_screen_recording_authorization_status")
public func ultravox_macos_bridge_screen_recording_authorization_status() -> Int32 {
    CGPreflightScreenCaptureAccess() ? 1 : 0
}

/// Requests Screen Recording access only when the user explicitly asks.
@_cdecl("ultravox_macos_bridge_request_screen_recording_access")
public func ultravox_macos_bridge_request_screen_recording_access() -> Int32 {
    CGRequestScreenCaptureAccess() ? 1 : 0
}

// MARK: - Meeting capture

@_cdecl("ultravox_macos_bridge_start_meeting_capture")
public func ultravox_macos_bridge_start_meeting_capture(
    _ path: UnsafePointer<CChar>?,
    _ includeMicrophone: Int32,
    _ errorOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?
) -> Int32 {
    errorOut?.pointee = nil
    guard #available(macOS 15.0, *) else {
        errorOut?.pointee = "Meeting mode requires macOS 15 or later.".duplicateAsCChar()
        return 0
    }
    let rawPath = path.map { String(cString: $0) } ?? ""
    guard !rawPath.isEmpty else {
        errorOut?.pointee = "Meeting recording path is empty.".duplicateAsCChar()
        return 0
    }
    do {
        try runAsyncAndBlock {
            try await MeetingCaptureManager.shared.start(
                outputURL: URL(fileURLWithPath: rawPath),
                captureMicrophone: includeMicrophone != 0
            )
        }
        return 1
    } catch {
        errorOut?.pointee = error.localizedDescription.duplicateAsCChar()
        return 0
    }
}

@_cdecl("ultravox_macos_bridge_stop_meeting_capture")
public func ultravox_macos_bridge_stop_meeting_capture(
    _ outputPath: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?,
    _ errorOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?
) -> Int32 {
    outputPath?.pointee = nil
    errorOut?.pointee = nil
    guard #available(macOS 15.0, *) else {
        errorOut?.pointee = "Meeting mode requires macOS 15 or later.".duplicateAsCChar()
        return 0
    }
    do {
        let audioURL = try runAsyncAndBlock {
            try await MeetingCaptureManager.shared.stop()
        }
        outputPath?.pointee = audioURL.path.duplicateAsCChar()
        return 1
    } catch {
        errorOut?.pointee = error.localizedDescription.duplicateAsCChar()
        return 0
    }
}

@_cdecl("ultravox_macos_bridge_set_meeting_capture_failure_callback")
public func ultravox_macos_bridge_set_meeting_capture_failure_callback(
    _ callback: (@convention(c) (UnsafePointer<CChar>?) -> Void)?
) {
    guard #available(macOS 15.0, *) else { return }
    MeetingCaptureManager.shared.setFailureCallback(callback)
}

// MARK: - Accessibility and focus targeting

/// Returns whether UltraVox can inspect and edit the focused accessibility element.
/// Passing a non-zero prompt value asks macOS to show its permission prompt.
@_cdecl("ultravox_macos_bridge_is_accessibility_trusted")
public func ultravox_macos_bridge_is_accessibility_trusted(_ prompt: Int32) -> Int32 {
    let trusted = runOnMainActor {
        guard prompt != 0 else {
            return AXIsProcessTrusted()
        }
        let options = [
            kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true
        ] as CFDictionary
        return AXIsProcessTrustedWithOptions(options)
    }
    return trusted ? 1 : 0
}

/// Returns the current caret position without changing the insertion target.
@_cdecl("ultravox_macos_bridge_get_caret_position")
public func ultravox_macos_bridge_get_caret_position(
    _ xOut: UnsafeMutablePointer<Double>?,
    _ yOut: UnsafeMutablePointer<Double>?
) -> Int32 {
    let result = runOnMainActor {
        FocusTargetManager.currentCaretPosition()
    }
    guard let result else { return 0 }
    xOut?.pointee = result.x
    yOut?.pointee = result.y
    return 1
}

/// Captures the focused element for the current recording and returns the text
/// insertion caret when available, otherwise the current pointer position.
@_cdecl("ultravox_macos_bridge_capture_insertion_target")
public func ultravox_macos_bridge_capture_insertion_target(
    _ xOut: UnsafeMutablePointer<Double>?,
    _ yOut: UnsafeMutablePointer<Double>?
) -> Int32 {
    let capture = runOnMainActor {
        FocusTargetManager.shared.capture()
    }
    xOut?.pointee = capture.x
    yOut?.pointee = capture.y
    return capture.hasTarget ? 1 : 0
}

@_cdecl("ultravox_macos_bridge_clear_insertion_target")
public func ultravox_macos_bridge_clear_insertion_target() {
    runOnMainActor {
        FocusTargetManager.shared.clear()
    }
}

/// Reports the frontmost application bundle identifier via NSWorkspace.
/// Returns NULL when unavailable or unknown; the caller frees a non-NULL
/// result with ultravox_macos_bridge_free_string.
@_cdecl("ultravox_macos_bridge_frontmost_application_bundle_id")
public func ultravox_macos_bridge_frontmost_application_bundle_id()
    -> UnsafeMutablePointer<CChar>?
{
    let bundleIdentifier = runOnMainActor {
        NSWorkspace.shared.frontmostApplication?.bundleIdentifier
    }
    return bundleIdentifier?.duplicateAsCChar()
}

// MARK: - CGEvent paste

/// Pastes the given text via the clipboard and a Cmd+V CGEvent keyboard event.
/// Saves the current clipboard contents before pasting and restores them after
/// a short delay. The clipboard/keycode work must run on the main thread; the
/// whole operation is dispatched there so Rust callers on tokio worker threads
/// do not crash against TIS/NSPasteboard thread assertions. Returns 1 on
/// success, 0 if the paste could not be posted.
@_cdecl("ultravox_macos_bridge_paste_text")
public func ultravox_macos_bridge_paste_text(_ text: UnsafePointer<CChar>?) -> Int32 {
    let rawText = text.map { String(cString: $0) } ?? ""
    guard !rawText.isEmpty else { return 0 }
    let inserted = runOnMainActor {
        FocusTargetManager.shared.insert(rawText)
    }
    return inserted ? 1 : 0
}

// MARK: - Modifier-only hotkey

/// Starts a CGEventTap that listens for one physical modifier key. The key is
/// always press-and-hold: key down starts recording and key up stops it.
@_cdecl("ultravox_macos_bridge_start_modifier_hotkey")
public func ultravox_macos_bridge_start_modifier_hotkey(_ modifier: UnsafePointer<CChar>?) -> Int32 {
    let rawModifier = modifier.map { String(cString: $0) } ?? "none"
    let started = runOnMainActor {
        ModifierOnlyMonitor.shared.start(modifier: rawModifier)
    }
    return started ? 1 : 0
}

/// Stops the modifier-only hotkey tap.
@_cdecl("ultravox_macos_bridge_stop_modifier_hotkey")
public func ultravox_macos_bridge_stop_modifier_hotkey() -> Int32 {
    runOnMainActor {
        ModifierOnlyMonitor.shared.stop()
    }
    return 1
}

// MARK: - Key combination hotkey with hold-to-record

/// Starts a CGEventTap that listens for a key combination hotkey.
/// Set `hold_to_record` to true to receive separate key-down and key-up events
/// through the callback registered with `ultravox_macos_bridge_set_key_combination_callback`.
@_cdecl("ultravox_macos_bridge_start_key_combination_hotkey")
public func ultravox_macos_bridge_start_key_combination_hotkey(
    _ combo: UnsafePointer<CChar>?,
    _ holdToRecord: Int32
) -> Int32 {
    let rawCombo = combo.map { String(cString: $0) } ?? "Option+Backtick"
    let started = runOnMainActor {
        KeyCombinationMonitor.shared.start(
            combo: rawCombo,
            holdToRecord: holdToRecord != 0
        )
    }
    return started ? 1 : 0
}

/// Stops the key combination hotkey tap.
@_cdecl("ultravox_macos_bridge_stop_key_combination_hotkey")
public func ultravox_macos_bridge_stop_key_combination_hotkey() -> Int32 {
    runOnMainActor {
        KeyCombinationMonitor.shared.stop()
    }
    return 1
}

/// Registers a Rust callback that receives key-down (0) and key-up (1) events.
@_cdecl("ultravox_macos_bridge_set_key_combination_callback")
public func ultravox_macos_bridge_set_key_combination_callback(
    _ callback: (@convention(c) (Int32, UnsafePointer<CChar>?) -> Void)?
) {
    KeyCombinationMonitor.shared.setCallback(callback)
}

/// Starts a separate key-combination tap for toggling meeting mode.
@_cdecl("ultravox_macos_bridge_start_meeting_hotkey")
public func ultravox_macos_bridge_start_meeting_hotkey(
    _ combo: UnsafePointer<CChar>?
) -> Int32 {
    let rawCombo = combo.map { String(cString: $0) } ?? "Control+M"
    let started = runOnMainActor {
        KeyCombinationMonitor.meeting.start(combo: rawCombo, holdToRecord: false)
    }
    return started ? 1 : 0
}

@_cdecl("ultravox_macos_bridge_stop_meeting_hotkey")
public func ultravox_macos_bridge_stop_meeting_hotkey() -> Int32 {
    runOnMainActor {
        KeyCombinationMonitor.meeting.stop()
    }
    return 1
}

@_cdecl("ultravox_macos_bridge_set_meeting_hotkey_callback")
public func ultravox_macos_bridge_set_meeting_hotkey_callback(
    _ callback: (@convention(c) (Int32, UnsafePointer<CChar>?) -> Void)?
) {
    KeyCombinationMonitor.meeting.setCallback(callback)
}

// MARK: - Nonactivating indicator

/// Shows a nonactivating indicator panel near the given point.
/// The panel is borderless, does not activate the app, and displays a small
/// recording indicator.
@_cdecl("ultravox_macos_bridge_show_indicator")
public func ultravox_macos_bridge_show_indicator(x: Double, y: Double) -> Int32 {
    runOnMainActor {
        IndicatorManager.shared.show(at: NSPoint(x: x, y: y))
    }
    return 1
}

/// Updates the visible indicator without moving or activating it.
@_cdecl("ultravox_macos_bridge_set_indicator_state")
public func ultravox_macos_bridge_set_indicator_state(_ state: UnsafePointer<CChar>?) -> Int32 {
    let rawState = state.map { String(cString: $0) } ?? "recording"
    runOnMainActor {
        IndicatorManager.shared.update(state: rawState)
    }
    return 1
}

/// Hides the nonactivating indicator panel.
@_cdecl("ultravox_macos_bridge_hide_indicator")
public func ultravox_macos_bridge_hide_indicator() -> Int32 {
    runOnMainActor {
        IndicatorManager.shared.hide()
    }
    return 1
}

// MARK: - Latency-critical activity

/// Holds ProcessInfo activities by token. While one is held, macOS does not
/// App Nap UltraVox, so capture and transcription keep pace with the
/// microphone when every UltraVox window is hidden.
private final class LatencyActivityRegistry: @unchecked Sendable {
    static let shared = LatencyActivityRegistry()
    private let lock = NSLock()
    private var nextToken: Int64 = 1
    private var activities: [Int64: NSObjectProtocol] = [:]

    func begin(reason: String) -> Int64 {
        let activity = ProcessInfo.processInfo.beginActivity(
            options: [.userInitiated, .latencyCritical],
            reason: reason
        )
        lock.lock()
        defer { lock.unlock() }
        let token = nextToken
        nextToken += 1
        activities[token] = activity
        return token
    }

    func end(token: Int64) -> Bool {
        lock.lock()
        let activity = activities.removeValue(forKey: token)
        lock.unlock()
        guard let activity else { return false }
        ProcessInfo.processInfo.endActivity(activity)
        return true
    }
}

/// Begins a user-initiated, latency-critical activity. Returns a token for
/// ultravox_macos_bridge_end_latency_activity.
@_cdecl("ultravox_macos_bridge_begin_latency_activity")
public func ultravox_macos_bridge_begin_latency_activity(_ reason: UnsafePointer<CChar>?) -> Int64 {
    let text = reason.map { String(cString: $0) } ?? "UltraVox dictation"
    return LatencyActivityRegistry.shared.begin(reason: text)
}

/// Ends the activity for token. Returns 1 when it was held, 0 otherwise.
@_cdecl("ultravox_macos_bridge_end_latency_activity")
public func ultravox_macos_bridge_end_latency_activity(_ token: Int64) -> Int32 {
    LatencyActivityRegistry.shared.end(token: token) ? 1 : 0
}

// MARK: - FluidAudio / CoreML transcription

/// Transcribes the audio file at the given path using FluidAudio.
/// Defaults to the English v2 model to match the UltraVox app default.
@_cdecl("ultravox_macos_bridge_transcribe_file")
public func ultravox_macos_bridge_transcribe_file(
    _ path: UnsafePointer<CChar>?,
    _ textOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?
) -> Int32 {
    return ultravox_macos_bridge_transcribe_file_with_version(path, "v2", nil, nil, textOut)
}

/// Transcribes the audio file at the given path using FluidAudio, selecting the
/// model version ("v2" for English, "v3" for multilingual). recording_id is
/// optional; when provided, it enables targeted cancellation of this specific
/// transcription. directory is optional and selects a custom model cache.
@_cdecl("ultravox_macos_bridge_transcribe_file_with_version")
public func ultravox_macos_bridge_transcribe_file_with_version(
    _ path: UnsafePointer<CChar>?,
    _ version: UnsafePointer<CChar>?,
    _ recordingId: UnsafePointer<CChar>?,
    _ directory: UnsafePointer<CChar>?,
    _ textOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?
) -> Int32 {
    let rawPath = path.map { String(cString: $0) } ?? ""
    guard !rawPath.isEmpty else {
        textOut?.pointee = nil
        return 0
    }

    let fileManager = FileManager.default
    guard fileManager.fileExists(atPath: rawPath),
          let attrs = try? fileManager.attributesOfItem(atPath: rawPath),
          let size = attrs[.size] as? NSNumber,
          size.int64Value > 0
    else {
        textOut?.pointee = nil
        return 0
    }

    let url = URL(fileURLWithPath: rawPath)
    let versionString = version.map { String(cString: $0) } ?? "v2"
    let recordingIdString = recordingId.map { String(cString: $0) } ?? ""
    let directoryURL = directory
        .map { String(cString: $0) }
        .flatMap { $0.isEmpty ? nil : URL(fileURLWithPath: $0, isDirectory: true) }

#if canImport(FluidAudio)
    do {
        let text = try runAsyncAndBlock {
            try await FluidAudioTranscriptionEngine.shared.transcribe(
                url: url,
                versionString: versionString,
                recordingId: recordingIdString,
                directory: directoryURL
            )
        }
        textOut?.pointee = text.duplicateAsCChar()
        return 1
    } catch {
        print("[UltraVoxMacOSBridge] transcription failed: \(error)")
        textOut?.pointee = nil
        return 0
    }
#else
    print("[UltraVoxMacOSBridge] FluidAudio not available on this platform")
    textOut?.pointee = nil
    return 0
#endif
}

/// Cancels the active FluidAudio transcription for the given recording identity.
/// Returns 1 when a matching in-flight transcription was found and cancelled;
/// 0 otherwise. The shared engine will never cancel a different recording.
@_cdecl("ultravox_macos_bridge_cancel_transcription")
public func ultravox_macos_bridge_cancel_transcription(
    _ recordingId: UnsafePointer<CChar>?
) -> Int32 {
    let recordingIdString = recordingId.map { String(cString: $0) } ?? ""
    guard !recordingIdString.isEmpty else {
        return 0
    }
#if canImport(FluidAudio)
    do {
        let cancelled = try runAsyncAndBlock {
            await FluidAudioTranscriptionEngine.shared.cancelTranscription(
                recordingId: recordingIdString
            )
        }
        return cancelled ? 1 : 0
    } catch {
        print("[UltraVoxMacOSBridge] cancel_transcription failed: \(error)")
        return 0
    }
#else
    return 0
#endif
}

/// Downloads and loads the FluidAudio model for the given version ("v2" or "v3").
/// Returns 1 on success and 0 on failure. The download is performed by the
/// FluidAudio SDK, so it may take a while for the multi-gigabyte CoreML assets.
@_cdecl("ultravox_macos_bridge_prepare_model")
public func ultravox_macos_bridge_prepare_model(
    _ version: UnsafePointer<CChar>?,
    _ directory: UnsafePointer<CChar>?
) -> Int32 {
    let versionString = version.map { String(cString: $0) } ?? "v2"
    let directoryURL = directory.map { URL(fileURLWithPath: String(cString: $0), isDirectory: true) }
#if canImport(FluidAudio)
    do {
        try runAsyncAndBlock {
            try await FluidAudioTranscriptionEngine.shared.ensureLoaded(
                versionString: versionString,
                directory: directoryURL
            )
        }
        return 1
    } catch {
        print("[UltraVoxMacOSBridge] prepare_model failed: \(error)")
        return 0
    }
#else
    print("[UltraVoxMacOSBridge] FluidAudio not available on this platform")
    return 0
#endif
}

/// Returns 1 if the FluidAudio model for the given version is already
/// downloaded to the standard cache directory, otherwise 0.
@_cdecl("ultravox_macos_bridge_is_model_downloaded")
public func ultravox_macos_bridge_is_model_downloaded(
    _ version: UnsafePointer<CChar>?,
    _ directory: UnsafePointer<CChar>?
) -> Int32 {
    let versionString = version.map { String(cString: $0) } ?? "v2"
#if canImport(FluidAudio)
    let v = asrVersion(from: versionString)
    let cacheDirectory = directory
        .map { URL(fileURLWithPath: String(cString: $0), isDirectory: true) }
        ?? AsrModels.defaultCacheDirectory(for: v)
    return AsrModels.modelsExist(at: cacheDirectory, version: v) ? 1 : 0
#else
    return 0
#endif
}

/// Returns the latest model preparation progress in [0, 1].
@_cdecl("ultravox_macos_bridge_get_model_progress")
public func ultravox_macos_bridge_get_model_progress(
    _ version: UnsafePointer<CChar>?
) -> Double {
    let versionString = version.map { String(cString: $0) } ?? "v2"
    return ModelProgressStore.shared.value(for: versionString)
}

// MARK: - Helpers

extension String {
    fileprivate func duplicateAsCChar() -> UnsafeMutablePointer<CChar> {
        guard let duplicate = withCString({ strdup($0) }) else {
            fatalError("Unable to allocate C string")
        }
        return duplicate
    }
}

internal enum FocusUtils {
    private static func primaryScreen() -> NSScreen? {
        let mainDisplayID = CGMainDisplayID()
        return NSScreen.screens.first { screen in
            guard let number = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber
            else { return false }
            return CGDirectDisplayID(number.uint32Value) == mainDisplayID
        }
    }

    /// Converts an AX top-left-origin screen point to the AppKit bottom-left-origin
    /// screen coordinate system. The AX coordinate space is global, so the flip
    /// is anchored against the primary display's height regardless of which screen
    /// contains the resulting point. Tests can inject an explicit frame.
    static func convertAXPointToCocoa(
        _ axPoint: CGPoint,
        primaryScreenFrame: CGRect? = nil
    ) -> NSPoint {
        let frame = primaryScreenFrame ?? primaryScreen()?.frame
        guard let frame else {
            return NSPoint(x: axPoint.x, y: axPoint.y)
        }
        return NSPoint(x: axPoint.x, y: frame.maxY - axPoint.y)
    }
}

private enum KeyboardLayout {
    static func findKeycodeForCharacter(_ char: Character) -> CGKeyCode? {
        guard let inputSource = TISCopyCurrentKeyboardInputSource()?.takeRetainedValue(),
              let layoutDataPtr = TISGetInputSourceProperty(inputSource, kTISPropertyUnicodeKeyLayoutData)
        else { return nil }

        let layoutData = unsafeBitCast(layoutDataPtr, to: CFData.self)
        let keyboardLayout = unsafeBitCast(
            CFDataGetBytePtr(layoutData),
            to: UnsafePointer<UCKeyboardLayout>.self
        )

        let targetLower = char.lowercased()
        for keycode: UInt16 in 0 ... 50 {
            var deadKeyState: UInt32 = 0
            var chars = [UniChar](repeating: 0, count: 4)
            var length: Int = 0

            let status = UCKeyTranslate(
                keyboardLayout,
                keycode,
                UInt16(kUCKeyActionDisplay),
                0,
                UInt32(LMGetKbdType()),
                UInt32(kUCKeyTranslateNoDeadKeysBit),
                &deadKeyState,
                4,
                &length,
                &chars
            )

            if status == noErr && length > 0 {
                let resultChar = Character(UnicodeScalar(chars[0])!)
                if resultChar.lowercased() == targetLower {
                    return CGKeyCode(keycode)
                }
            }
        }
        return nil
    }
}

private enum ClipboardUtil {
    typealias SavedContents = ([NSPasteboard.PasteboardType: Any], [NSPasteboard.PasteboardType])

    static func insertWithCommandV(_ text: String) -> Bool {
        let pasteboard = NSPasteboard.general
        let savedContents = saveCurrentPasteboardContents(pasteboard)

        pasteboard.declareTypes([.string], owner: nil)
        pasteboard.setString(text, forType: .string)

        let keyCodeV = KeyboardLayout.findKeycodeForCharacter("v") ?? 9
        guard let source = CGEventSource(stateID: .combinedSessionState),
              let keyDown = CGEvent(keyboardEventSource: source, virtualKey: keyCodeV, keyDown: true),
              let keyUp = CGEvent(keyboardEventSource: source, virtualKey: keyCodeV, keyDown: false)
        else {
            restore(savedContents, to: pasteboard)
            return false
        }

        keyDown.flags = .maskCommand
        keyUp.flags = .maskCommand
        keyDown.post(tap: .cghidEventTap)
        keyUp.post(tap: .cghidEventTap)
        // The target reads the pasteboard asynchronously (Electron apps can take
        // well over 300 ms). Restore later without blocking the main run loop,
        // and never clobber something the user copied in the meantime.
        let transcriptChangeCount = pasteboard.changeCount
        nonisolated(unsafe) let saved = savedContents
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.2) {
            let general = NSPasteboard.general
            guard general.changeCount == transcriptChangeCount else { return }
            restore(saved, to: general)
        }
        return true
    }

    static func saveCurrentPasteboardContents(_ pasteboard: NSPasteboard) -> SavedContents? {
        let types = pasteboard.types ?? []
        guard !types.isEmpty else { return nil }

        var saved: [NSPasteboard.PasteboardType: Any] = [:]
        for type in types {
            if let data = pasteboard.data(forType: type) {
                saved[type] = data
            } else if let string = pasteboard.string(forType: type) {
                saved[type] = string
            } else if let urls = pasteboard.propertyList(forType: type) as? [String] {
                saved[type] = urls
            }
        }
        return saved.isEmpty ? nil : (saved, types)
    }

    static func restorePasteboardContents(_ pasteboard: NSPasteboard, _ contents: SavedContents) {
        let (saved, types) = contents
        pasteboard.declareTypes(types, owner: nil)
        for (type, content) in saved {
            if let data = content as? Data {
                pasteboard.setData(data, forType: type)
            } else if let string = content as? String {
                pasteboard.setString(string, forType: type)
            } else if let urls = content as? [String] {
                pasteboard.setPropertyList(urls, forType: type)
            }
        }
    }

    private static func restore(_ contents: SavedContents?, to pasteboard: NSPasteboard) {
        if let contents {
            restorePasteboardContents(pasteboard, contents)
        } else {
            pasteboard.clearContents()
        }
    }
}

private struct FocusCapture: Sendable {
    let x: Double
    let y: Double
    let hasTarget: Bool
}

@MainActor
private final class FocusTargetManager {
    static let shared = FocusTargetManager()

    private var element: AXUIElement?
    private var processIdentifier: pid_t?

    private init() {}

    static func currentCaretPosition() -> (x: Double, y: Double)? {
        guard let element = focusedElement() else { return nil }
        return caretPosition(for: element)
    }

    func capture() -> FocusCapture {
        clear()
        let pointerPosition = NSEvent.mouseLocation
        // Remember the frontmost app even when Accessibility cannot expose a
        // focused element, so delivery can still fall back to Cmd+V there.
        if let frontmost = NSWorkspace.shared.frontmostApplication,
           frontmost.processIdentifier != ProcessInfo.processInfo.processIdentifier {
            processIdentifier = frontmost.processIdentifier
        }
        guard AXIsProcessTrusted(), let focused = Self.focusedElement() else {
            return FocusCapture(x: pointerPosition.x, y: pointerPosition.y, hasTarget: false)
        }

        element = focused
        var pid: pid_t = 0
        if AXUIElementGetPid(focused, &pid) == .success {
            processIdentifier = pid
        }

        let indicatorPosition = Self.caretPosition(for: focused)
            ?? (x: Double(pointerPosition.x), y: Double(pointerPosition.y))
        return FocusCapture(
            x: indicatorPosition.x,
            y: indicatorPosition.y,
            hasTarget: true
        )
    }

    func insert(_ text: String) -> Bool {
        defer { clear() }
        guard AXIsProcessTrusted() else {
            NSLog("[UltraVoxMacOSBridge] paste: accessibility not trusted")
            return false
        }

        if let element {
            // Many Chromium/Electron and web fields report success for
            // kAXSelectedText without changing their value. Only trust the
            // accessibility path when the field's value observably changed.
            let before = Self.stringValue(of: element)
            let result = AXUIElementSetAttributeValue(
                element,
                kAXSelectedTextAttribute as CFString,
                text as CFTypeRef
            )
            if result == .success {
                let after = Self.stringValue(of: element)
                if before == nil || after == nil || before != after {
                    NSLog("[UltraVoxMacOSBridge] paste: inserted via accessibility (verified=%@)", (before != nil && after != nil) ? "yes" : "unverifiable")
                    return true
                }
                NSLog("[UltraVoxMacOSBridge] accessibility insert reported success without a change; using Cmd+V")
            }
        }

        guard let processIdentifier,
              let application = NSRunningApplication(processIdentifier: processIdentifier),
              !application.isTerminated
        else {
            NSLog("[UltraVoxMacOSBridge] paste: no target application for Cmd+V fallback")
            return false
        }
        NSLog("[UltraVoxMacOSBridge] paste: Cmd+V fallback into %@", application.bundleIdentifier ?? "unknown")
        if !application.isActive {
            application.activate()
        }
        if let element {
            _ = AXUIElementSetAttributeValue(
                element,
                kAXFocusedAttribute as CFString,
                kCFBooleanTrue
            )
        }
        RunLoop.current.run(until: Date().addingTimeInterval(0.08))
        return ClipboardUtil.insertWithCommandV(text)
    }

    private static func stringValue(of element: AXUIElement) -> String? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, kAXValueAttribute as CFString, &value) == .success
        else { return nil }
        return value as? String
    }

    func clear() {
        element = nil
        processIdentifier = nil
    }

    private static func focusedElement() -> AXUIElement? {
        let systemElement = AXUIElementCreateSystemWide()
        var focusedElement: CFTypeRef?
        let error = AXUIElementCopyAttributeValue(
            systemElement,
            kAXFocusedUIElementAttribute as CFString,
            &focusedElement
        )
        guard error == .success, let focusedElement else { return nil }
        return (focusedElement as! AXUIElement)
    }

    private static func caretPosition(for element: AXUIElement) -> (x: Double, y: Double)? {
        var selectedTextRange: AnyObject?
        let rangeError = AXUIElementCopyAttributeValue(
            element,
            kAXSelectedTextRangeAttribute as CFString,
            &selectedTextRange
        )
        guard rangeError == .success, let selectedTextRange else { return nil }
        let selectedRangeValue = selectedTextRange as! AXValue

        var selectedRange = CFRange()
        guard AXValueGetValue(selectedRangeValue, .cfRange, &selectedRange) else { return nil }
        var insertionRange = CFRange(
            location: selectedRange.location + selectedRange.length,
            length: 0
        )
        guard let insertionRangeValue = AXValueCreate(.cfRange, &insertionRange) else { return nil }

        var caretBounds: CFTypeRef?
        let boundsError = AXUIElementCopyParameterizedAttributeValue(
            element,
            kAXBoundsForRangeParameterizedAttribute as CFString,
            insertionRangeValue,
            &caretBounds
        )
        guard boundsError == .success, let caretBounds else { return nil }

        let value = caretBounds as! AXValue
        var rect = CGRect.zero
        guard AXValueGetValue(value, .cgRect, &rect),
              rect.height > 0,
              rect.origin.x.isFinite,
              rect.origin.y.isFinite
        else {
            return nil
        }
        let point = FocusUtils.convertAXPointToCocoa(rect.origin)
        return (Double(point.x), Double(point.y))
    }
}

private final class ModifierOnlyMonitor: @unchecked Sendable {
    static let shared = ModifierOnlyMonitor()

    private let lock = NSRecursiveLock()
    private var eventTap: CFMachPort?
    private var runLoopSource: CFRunLoopSource?
    private var modifier: PhysicalModifier?
    private var pressedKeyCodes = Set<UInt16>()

    private init() {}

    @discardableResult
    func start(modifier rawValue: String) -> Bool {
        lock.lock()
        defer { lock.unlock() }

        let lowered = rawValue.lowercased()
        // Legacy Shift settings (leftShift/rightShift/shift) are treated as
        // disabled so that an old persisted value does not break hotkey setup
        // now that Shift is no longer monitored as a modifier-only hotkey.
        if lowered == "none" || lowered == "shift" || lowered == "leftshift" || lowered == "rightshift" {
            stop()
            return true
        }

        guard let parsed = PhysicalModifier(rawValue: rawValue) else {
            stop()
            return false
        }

        stop()
        modifier = parsed
        pressedKeyCodes.removeAll()

        let eventMask = 1 << CGEventType.flagsChanged.rawValue
        guard let tap = CGEvent.tapCreate(
            tap: .cgSessionEventTap,
            place: .headInsertEventTap,
            options: .defaultTap,
            eventsOfInterest: CGEventMask(eventMask),
            callback: { _, type, event, refcon -> Unmanaged<CGEvent>? in
                guard let refcon else { return Unmanaged.passUnretained(event) }
                let monitor = Unmanaged<ModifierOnlyMonitor>.fromOpaque(refcon).takeUnretainedValue()
                if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
                    monitor.reenableTap()
                    return Unmanaged.passUnretained(event)
                }
                return monitor.handle(event: event)
                    ? nil
                    : Unmanaged.passUnretained(event)
            },
            userInfo: Unmanaged.passUnretained(self).toOpaque()
        ) else {
            modifier = nil
            return false
        }

        eventTap = tap
        runLoopSource = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0)
        guard let source = runLoopSource else {
            eventTap = nil
            modifier = nil
            return false
        }

        CFRunLoopAddSource(CFRunLoopGetCurrent(), source, .commonModes)
        CGEvent.tapEnable(tap: tap, enable: true)
        print("[UltraVoxMacOSBridge] monitoring modifier \(rawValue)")
        return true
    }

    func stop() {
        lock.lock()
        defer { lock.unlock() }
        if let tap = eventTap {
            CGEvent.tapEnable(tap: tap, enable: false)
            if let source = runLoopSource {
                CFRunLoopRemoveSource(CFRunLoopGetCurrent(), source, .commonModes)
            }
        }
        eventTap = nil
        runLoopSource = nil
        modifier = nil
        pressedKeyCodes.removeAll()
    }

    private func reenableTap() {
        lock.lock()
        defer { lock.unlock() }
        if let tap = eventTap {
            CGEvent.tapEnable(tap: tap, enable: true)
        }
    }

    private func handle(event: CGEvent) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard let modifier = modifier else { return false }

        let keyCode = UInt16(event.getIntegerValueField(.keyboardEventKeycode))
        guard keyCode == modifier.keyCode else { return false }

        // Distinguish left/right modifiers by physical key code rather than by
        // CGEventFlags, which only has a single flag bit for both sides. Track
        // the pressed key-code set so a release of one side is detected even
        // when the opposite modifier of the same flag is still held.
        if pressedKeyCodes.remove(keyCode) != nil {
            KeyCombinationMonitor.shared.notify(event: 1, raw: modifier.rawValue)
        } else {
            pressedKeyCodes.insert(keyCode)
            KeyCombinationMonitor.shared.notify(event: 0, raw: modifier.rawValue)
        }
        return true
    }
}

internal struct PhysicalModifier {
    let rawValue: String
    let keyCode: UInt16

    init?(rawValue: String) {
        switch rawValue.lowercased() {
        case "leftoption":
            self = Self(rawValue: "leftOption", keyCode: 58)
        case "rightoption":
            self = Self(rawValue: "rightOption", keyCode: 61)
        case "rightcommand":
            self = Self(rawValue: "rightCommand", keyCode: 54)
        default:
            return nil
        }
    }

    private init(rawValue: String, keyCode: UInt16) {
        self.rawValue = rawValue
        self.keyCode = keyCode
    }
}

private final class KeyCombinationMonitor: @unchecked Sendable {
    static let shared = KeyCombinationMonitor(identifier: 1)
    static let meeting = KeyCombinationMonitor(identifier: 2)

    private static let signature: OSType = 0x44494354 // "DICT"

    private let lock = NSRecursiveLock()
    private let hotKeyID: EventHotKeyID
    private var hotKeyRef: EventHotKeyRef?
    private var eventHandler: EventHandlerRef?
    private var combo: KeyCombination?
    private var holdToRecord = false
    private var isKeyDown = false
    var callback: (@convention(c) (Int32, UnsafePointer<CChar>?) -> Void)?

    private init(identifier: UInt32) {
        hotKeyID = EventHotKeyID(signature: Self.signature, id: identifier)
    }

    func setCallback(_ callback: (@convention(c) (Int32, UnsafePointer<CChar>?) -> Void)?) {
        lock.lock()
        defer { lock.unlock() }
        self.callback = callback
    }

    @discardableResult
    func start(combo: String, holdToRecord: Bool) -> Bool {
        lock.lock()
        defer { lock.unlock() }

        guard let parsed = KeyCombination.parse(combo) else {
            print("[UltraVoxMacOSBridge] unable to parse key combination: \(combo)")
            return false
        }
        guard holdToRecord || parsed.modifier != nil else {
            print("[UltraVoxMacOSBridge] bare Space requires hold-to-record")
            return false
        }

        stop()
        self.combo = parsed
        self.holdToRecord = holdToRecord
        isKeyDown = false

        var eventTypes = [
            EventTypeSpec(
                eventClass: OSType(kEventClassKeyboard),
                eventKind: UInt32(kEventHotKeyPressed)
            ),
            EventTypeSpec(
                eventClass: OSType(kEventClassKeyboard),
                eventKind: UInt32(kEventHotKeyReleased)
            ),
        ]
        let installStatus = InstallEventHandler(
            GetApplicationEventTarget(),
            { _, event, userInfo in
                guard let event, let userInfo else { return OSStatus(eventNotHandledErr) }
                let monitor = Unmanaged<KeyCombinationMonitor>
                    .fromOpaque(userInfo)
                    .takeUnretainedValue()
                return monitor.handle(event: event)
            },
            eventTypes.count,
            &eventTypes,
            Unmanaged.passUnretained(self).toOpaque(),
            &eventHandler
        )
        guard installStatus == noErr else {
            self.combo = nil
            print("[UltraVoxMacOSBridge] failed to install key combination handler: \(installStatus)")
            return false
        }

        let registerStatus = RegisterEventHotKey(
            UInt32(parsed.keyCode),
            parsed.modifier?.carbonMask ?? 0,
            hotKeyID,
            GetApplicationEventTarget(),
            0,
            &hotKeyRef
        )
        guard registerStatus == noErr else {
            if let eventHandler {
                RemoveEventHandler(eventHandler)
                self.eventHandler = nil
            }
            self.combo = nil
            print("[UltraVoxMacOSBridge] failed to register key combination: \(registerStatus)")
            return false
        }

        print("[UltraVoxMacOSBridge] monitoring \(combo)")
        return true
    }

    func stop() {
        lock.lock()
        defer { lock.unlock() }

        if let hotKeyRef {
            UnregisterEventHotKey(hotKeyRef)
            self.hotKeyRef = nil
        }
        if let eventHandler {
            RemoveEventHandler(eventHandler)
            self.eventHandler = nil
        }
        combo = nil
        isKeyDown = false
        print("[UltraVoxMacOSBridge] stopped key combination monitor")
    }

    private func handle(event: EventRef) -> OSStatus {
        var receivedID = EventHotKeyID()
        let parameterStatus = GetEventParameter(
            event,
            EventParamName(kEventParamDirectObject),
            EventParamType(typeEventHotKeyID),
            nil,
            MemoryLayout<EventHotKeyID>.size,
            nil,
            &receivedID
        )
        guard parameterStatus == noErr, receivedID.signature == hotKeyID.signature,
              receivedID.id == hotKeyID.id
        else {
            return OSStatus(eventNotHandledErr)
        }

        lock.lock()
        defer { lock.unlock() }
        guard combo != nil else { return OSStatus(eventNotHandledErr) }

        switch GetEventKind(event) {
        case UInt32(kEventHotKeyPressed):
            if !isKeyDown {
                isKeyDown = true
                notify(event: 0)
            }
        case UInt32(kEventHotKeyReleased):
            guard isKeyDown else { return noErr }
            isKeyDown = false
            if holdToRecord {
                notify(event: 1)
            }
        default:
            return OSStatus(eventNotHandledErr)
        }
        return noErr
    }

    private func notify(event: Int32) {
        guard let callback = callback, let combo = combo else { return }
        let cString = combo.raw.duplicateAsCChar()
        callback(event, cString)
        free(cString)
    }

    func notify(event: Int32, raw: String) {
        guard let callback else { return }
        let cString = raw.duplicateAsCChar()
        callback(event, cString)
        free(cString)
    }
}

struct KeyCombination {
    let raw: String
    let modifier: Modifier?
    let keyCode: UInt16

    enum Modifier {
        case command, option, control, shift

        var carbonMask: UInt32 {
            switch self {
            case .command: return UInt32(cmdKey)
            case .option: return UInt32(optionKey)
            case .control: return UInt32(controlKey)
            case .shift: return UInt32(shiftKey)
            }
        }
    }

    static func parse(_ combo: String) -> KeyCombination? {
        let parts = combo.split(separator: "+", omittingEmptySubsequences: false)
            .map { $0.trimmingCharacters(in: .whitespaces).lowercased() }
        if parts.count == 1, parts[0] == "space" {
            return KeyCombination(raw: combo, modifier: nil, keyCode: 49)
        }
        guard parts.count == 2,
              let modifier = parseModifier(parts[0]),
              let keyCode = keyCodeForName(parts[1])
        else { return nil }
        return KeyCombination(raw: combo, modifier: modifier, keyCode: keyCode)
    }

    private static func parseModifier(_ name: String) -> Modifier? {
        switch name {
        case "command", "cmd": return .command
        case "option", "opt", "alt": return .option
        case "control", "ctrl": return .control
        case "shift": return .shift
        default: return nil
        }
    }

    private static func keyCodeForName(_ name: String) -> UInt16? {
        switch name {
        case "backtick", "grave", "`": return 50
        case "escape", "esc": return 53
        case "space": return 49
        case "return", "enter": return 36
        case "a": return 0
        case "b": return 11
        case "c": return 8
        case "d": return 2
        case "e": return 14
        case "f": return 3
        case "g": return 5
        case "h": return 4
        case "i": return 34
        case "j": return 38
        case "k": return 40
        case "l": return 37
        case "m": return 46
        case "n": return 45
        case "o": return 31
        case "p": return 35
        case "q": return 12
        case "r": return 15
        case "s": return 1
        case "t": return 17
        case "u": return 32
        case "v": return 9
        case "w": return 13
        case "x": return 7
        case "y": return 16
        case "z": return 6
        case "0": return 29
        case "1": return 18
        case "2": return 19
        case "3": return 20
        case "4": return 21
        case "5": return 23
        case "6": return 22
        case "7": return 26
        case "8": return 28
        case "9": return 25
        default: return nil
        }
    }
}

private enum IndicatorState: Equatable {
    case recording
    case transcribing
    case inserted
    case failed
    case pasteFailed

    init(rawValue: String) {
        switch rawValue.lowercased() {
        case "transcribing": self = .transcribing
        case "inserted": self = .inserted
        case "failed": self = .failed
        case "paste-failed": self = .pasteFailed
        default: self = .recording
        }
    }

    var label: String {
        switch self {
        case .recording: return "Listening"
        case .transcribing: return "Transcribing"
        case .inserted: return "Inserted"
        case .failed: return "Transcription failed"
        case .pasteFailed: return "Copied \u{2014} paste with \u{2318}V"
        }
    }

    var color: NSColor {
        switch self {
        case .recording: return NSColor(calibratedRed: 1.0, green: 0.33, blue: 0.38, alpha: 1)
        case .transcribing: return NSColor(calibratedRed: 0.55, green: 0.72, blue: 1.0, alpha: 1)
        case .inserted: return NSColor(calibratedRed: 0.36, green: 0.84, blue: 0.52, alpha: 1)
        case .failed, .pasteFailed: return NSColor(calibratedRed: 1.0, green: 0.68, blue: 0.28, alpha: 1)
        }
    }

    /// Terminal states stay readable briefly before the panel fades out.
    var holdBeforeHide: TimeInterval {
        switch self {
        case .inserted: return 0.55
        case .failed, .pasteFailed: return 1.6
        default: return 0
        }
    }
}

/// Pure geometry for placing the nonactivating indicator panel.
/// The panel is centered horizontally on the caret/pointer and placed above it
/// by default; if that would clip out of the visible frame, it flips below.
internal enum IndicatorGeometry {
    static func clampedOrigin(
        point: NSPoint,
        size: NSSize,
        visibleFrame: NSRect,
        verticalOffset: CGFloat,
        padding: CGFloat
    ) -> NSPoint {
        var x = point.x - size.width / 2
        let topY = point.y + verticalOffset
        let topEdge = topY + size.height + padding

        // Prefer above the caret; flip below if it would be clipped.
        let y: CGFloat
        if topEdge > visibleFrame.maxY {
            y = point.y - size.height - verticalOffset
        } else {
            y = topY
        }

        x = max(visibleFrame.minX + padding, min(x, visibleFrame.maxX - size.width - padding))
        let clampedY = max(visibleFrame.minY + padding, min(y, visibleFrame.maxY - size.height - padding))
        return NSPoint(x: x, y: clampedY)
    }
}

@MainActor
private final class IndicatorManager {
    static let shared = IndicatorManager()

    private let height: CGFloat = 30
    private let horizontalPadding: CGFloat = 8
    private let verticalOffset: CGFloat = 14
    private var panel: NSPanel?
    private var indicatorView: IndicatorView?
    private var anchor = NSPoint.zero
    /// Bumped on every show so a pending delayed hide never closes a newer session.
    private var generation = 0

    private init() {}

    private var reduceMotion: Bool {
        NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
    }

    func show(at point: NSPoint) {
        if panel == nil {
            let initial = NSSize(width: 120, height: height)
            let panel = NSPanel(
                contentRect: NSRect(origin: .zero, size: initial),
                styleMask: [.borderless, .nonactivatingPanel],
                backing: .buffered,
                defer: false
            )
            let indicatorView = IndicatorView(frame: NSRect(origin: .zero, size: initial))
            panel.isFloatingPanel = true
            panel.backgroundColor = .clear
            panel.isOpaque = false
            panel.hasShadow = true
            panel.ignoresMouseEvents = true
            panel.hidesOnDeactivate = false
            panel.level = .statusBar
            panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient]
            panel.animationBehavior = .none
            panel.contentView = indicatorView
            self.panel = panel
            self.indicatorView = indicatorView
        }

        guard let panel, let indicatorView else { return }
        generation += 1
        anchor = point
        indicatorView.reduceMotion = reduceMotion
        indicatorView.state = .recording
        layout(panel: panel, view: indicatorView)

        if reduceMotion {
            panel.alphaValue = 1
            panel.orderFrontRegardless()
            return
        }
        let target = panel.frame
        panel.alphaValue = 0
        panel.setFrame(target.offsetBy(dx: 0, dy: -4), display: false)
        panel.orderFrontRegardless()
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.16
            context.timingFunction = CAMediaTimingFunction(name: .easeOut)
            panel.animator().alphaValue = 1
            panel.animator().setFrame(target, display: true)
        }
    }

    func update(state rawValue: String) {
        guard let panel, let indicatorView else { return }
        let next = IndicatorState(rawValue: rawValue)
        switch next {
        case .transcribing, .inserted:
            // Success needs no message: the pill leaves as soon as dictation
            // stops and the text simply appears (operator, 2026-09-25). A
            // "Transcribing" flash over the focused app read as a glitch.
            guard panel.isVisible else { return }
            generation += 1
            if reduceMotion {
                panel.orderOut(nil)
            } else {
                fadeOut(panel, token: generation)
            }
        case .failed, .pasteFailed:
            // Failures still speak up, at the same spot, even after the pill
            // already left for the transcription.
            generation += 1
            indicatorView.state = next
            layout(panel: panel, view: indicatorView)
            panel.alphaValue = 1
            if !panel.isVisible { panel.orderFrontRegardless() }
        case .recording:
            guard panel.isVisible, next != indicatorView.state else { return }
            indicatorView.state = next
            layout(panel: panel, view: indicatorView)
        }
    }

    func hide() {
        guard let panel, panel.isVisible else { return }
        let hold = indicatorView?.state.holdBeforeHide ?? 0
        let token = generation
        DispatchQueue.main.asyncAfter(deadline: .now() + hold) { [weak self] in
            MainActor.assumeIsolated {
                guard let self, self.generation == token else { return }
                self.fadeOut(panel, token: token)
            }
        }
    }

    private func fadeOut(_ panel: NSPanel, token: Int) {
        if reduceMotion {
            panel.orderOut(nil)
            return
        }
        NSAnimationContext.runAnimationGroup({ context in
            context.duration = 0.18
            context.timingFunction = CAMediaTimingFunction(name: .easeIn)
            panel.animator().alphaValue = 0
        }, completionHandler: { [weak self] in
            MainActor.assumeIsolated {
                guard let self, self.generation == token else { return }
                panel.orderOut(nil)
                panel.alphaValue = 1
            }
        })
    }

    /// Sizes the pill to its label and keeps it centred on the caret anchor.
    private func layout(panel: NSPanel, view: IndicatorView) {
        let size = NSSize(width: view.preferredWidth, height: height)
        guard let screen = screenContaining(anchor) else { return }
        let origin = IndicatorGeometry.clampedOrigin(
            point: anchor,
            size: size,
            visibleFrame: screen.visibleFrame,
            verticalOffset: verticalOffset,
            padding: horizontalPadding
        )
        panel.setFrame(NSRect(origin: origin, size: size), display: true)
        view.frame = NSRect(origin: .zero, size: size)
    }

    private func screenContaining(_ point: NSPoint) -> NSScreen? {
        if let screen = NSScreen.screens.first(where: { $0.frame.contains(point) }) {
            return screen
        }
        // Points that fall in a gap between displays still belong to the closest
        // screen so clamping uses the correct visible frame.
        return NSScreen.screens.min {
            distance(from: point, to: $0.frame) < distance(from: point, to: $1.frame)
        }
    }

    private func distance(from point: NSPoint, to rect: NSRect) -> CGFloat {
        let dx = max(rect.minX - point.x, CGFloat(0), point.x - rect.maxX)
        let dy = max(rect.minY - point.y, CGFloat(0), point.y - rect.maxY)
        return hypot(dx, dy)
    }
}

/// The pill: a status glyph (pulsing dot, spinning arc or check) and a label.
private final class IndicatorView: NSView {
    private static let font = NSFont.systemFont(ofSize: 12, weight: .semibold)
    private let glyphSize: CGFloat = 12
    private let glyphLeading: CGFloat = 12
    private let labelLeading: CGFloat = 32
    private let trailing: CGFloat = 14

    private let dotLayer = CALayer()
    private let ringLayer = CALayer()
    private let arcLayer = CAShapeLayer()
    private let checkLayer = CAShapeLayer()
    var reduceMotion = false

    var state: IndicatorState = .recording {
        didSet {
            needsDisplay = true
            applyGlyph()
        }
    }

    var preferredWidth: CGFloat {
        let text = state.label as NSString
        let width = text.size(withAttributes: [.font: Self.font]).width
        return ceil(labelLeading + width + trailing)
    }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        for layer in [ringLayer, dotLayer, arcLayer, checkLayer] {
            layer.contentsScale = NSScreen.main?.backingScaleFactor ?? 2
            self.layer?.addSublayer(layer)
        }
        arcLayer.fillColor = nil
        arcLayer.lineWidth = 2
        arcLayer.lineCap = .round
        checkLayer.fillColor = nil
        checkLayer.lineWidth = 2
        checkLayer.lineCap = .round
        checkLayer.lineJoin = .round
        applyGlyph()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    override var isFlipped: Bool { false }

    override func layout() {
        super.layout()
        applyGlyph()
    }

    private func applyGlyph() {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        let glyphRect = NSRect(
            x: glyphLeading,
            y: bounds.midY - glyphSize / 2,
            width: glyphSize,
            height: glyphSize
        )
        let color = state.color.cgColor
        for layer in [dotLayer, ringLayer, arcLayer, checkLayer] {
            layer.removeAllAnimations()
            layer.isHidden = true
        }

        switch state {
        case .recording, .failed, .pasteFailed:
            let dot = glyphRect.insetBy(dx: 2, dy: 2)
            dotLayer.frame = dot
            dotLayer.cornerRadius = dot.width / 2
            dotLayer.backgroundColor = color
            dotLayer.isHidden = false
            if state == .recording && !reduceMotion {
                ringLayer.frame = dot
                ringLayer.cornerRadius = dot.width / 2
                ringLayer.borderColor = color
                ringLayer.borderWidth = 1.5
                ringLayer.isHidden = false
                let grow = CABasicAnimation(keyPath: "transform.scale")
                grow.fromValue = 1
                grow.toValue = 2.2
                let fade = CABasicAnimation(keyPath: "opacity")
                fade.fromValue = 0.7
                fade.toValue = 0
                let ripple = CAAnimationGroup()
                ripple.animations = [grow, fade]
                ripple.duration = 1.2
                ripple.repeatCount = .infinity
                ripple.timingFunction = CAMediaTimingFunction(name: .easeOut)
                ringLayer.add(ripple, forKey: "ripple")

                let breathe = CABasicAnimation(keyPath: "opacity")
                breathe.fromValue = 1
                breathe.toValue = 0.55
                breathe.duration = 0.6
                breathe.autoreverses = true
                breathe.repeatCount = .infinity
                dotLayer.add(breathe, forKey: "breathe")
            }
        case .transcribing:
            arcLayer.frame = glyphRect
            arcLayer.strokeColor = color
            let path = CGMutablePath()
            path.addArc(
                center: CGPoint(x: glyphSize / 2, y: glyphSize / 2),
                radius: glyphSize / 2 - 1,
                startAngle: 0,
                endAngle: .pi * 1.4,
                clockwise: false
            )
            arcLayer.path = path
            arcLayer.isHidden = false
            if !reduceMotion {
                let spin = CABasicAnimation(keyPath: "transform.rotation.z")
                spin.fromValue = 0
                spin.toValue = -Double.pi * 2
                spin.duration = 0.9
                spin.repeatCount = .infinity
                arcLayer.add(spin, forKey: "spin")
            }
        case .inserted:
            checkLayer.frame = glyphRect
            checkLayer.strokeColor = color
            let path = CGMutablePath()
            path.move(to: CGPoint(x: 1.5, y: glyphSize * 0.5))
            path.addLine(to: CGPoint(x: glyphSize * 0.4, y: glyphSize * 0.18))
            path.addLine(to: CGPoint(x: glyphSize - 1, y: glyphSize * 0.85))
            checkLayer.path = path
            checkLayer.isHidden = false
            if !reduceMotion {
                let draw = CABasicAnimation(keyPath: "strokeEnd")
                draw.fromValue = 0
                draw.toValue = 1
                draw.duration = 0.22
                draw.timingFunction = CAMediaTimingFunction(name: .easeOut)
                checkLayer.add(draw, forKey: "draw")
            }
        }
        CATransaction.commit()
    }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)

        let backgroundRect = bounds.insetBy(dx: 0.5, dy: 0.5)
        let cornerRadius = backgroundRect.height / 2
        let background = NSBezierPath(
            roundedRect: backgroundRect,
            xRadius: cornerRadius,
            yRadius: cornerRadius
        )
        NSColor(calibratedWhite: 0.07, alpha: 0.94).setFill()
        background.fill()

        // Soft top highlight gives the pill a glass edge on any wallpaper.
        NSGraphicsContext.saveGraphicsState()
        background.addClip()
        let sheen = NSGradient(colors: [
            NSColor.white.withAlphaComponent(0.10),
            NSColor.white.withAlphaComponent(0.0)
        ])
        sheen?.draw(in: NSRect(x: 0, y: bounds.midY, width: bounds.width, height: bounds.height / 2), angle: 90)
        NSGraphicsContext.restoreGraphicsState()

        NSColor.white.withAlphaComponent(0.16).setStroke()
        background.lineWidth = 1
        background.stroke()

        let attributes: [NSAttributedString.Key: Any] = [
            .font: Self.font,
            .foregroundColor: NSColor.white.withAlphaComponent(0.94)
        ]
        let label = state.label as NSString
        let textSize = label.size(withAttributes: attributes)
        label.draw(
            at: NSPoint(x: labelLeading, y: bounds.midY - textSize.height / 2),
            withAttributes: attributes
        )
    }
}

// MARK: - On-device Voice Studio

#if canImport(FluidAudio)
private actor PocketTtsEngine {
    static let shared = PocketTtsEngine()
    private var managers: [String: PocketTtsManager] = [:]
    // Actor methods are reentrant across awaits. Hold a FIFO async gate over
    // entire operations because FluidAudio's synthesizer uses shared state.
    private var busy = false
    private var waiters: [CheckedContinuation<Void, Never>] = []

    private func acquire() async {
        if !busy { busy = true; return }
        await withCheckedContinuation { waiters.append($0) }
    }

    private func release() {
        if waiters.isEmpty { busy = false } else { waiters.removeFirst().resume() }
    }

    private func manager(language: String, int8: Bool) throws -> PocketTtsManager {
        guard let pack = PocketTtsLanguage(rawValue: language) else {
            throw PocketTTSError.processingFailed("Unsupported PocketTTS language pack: \(language)")
        }
        let key = "\(language):\(int8)"
        if let manager = managers[key] { return manager }
        let manager = PocketTtsManager(language: pack, precision: int8 ? .int8 : .fp16)
        managers[key] = manager
        return manager
    }

    func prepare(language: String, int8: Bool) async throws {
        await acquire()
        defer { release() }
        let manager = try manager(language: language, int8: int8)
        if !(await manager.isAvailable) { try await manager.initialize() }
    }

    func clone(reference: String, output: String) async throws {
        await acquire()
        defer { release() }
        // The shared Mimi encoder is language-independent; no synthesis model
        // download is needed for cloning alone.
        let manager = try manager(language: "english", int8: false)
        let voice = try await manager.cloneVoice(from: URL(fileURLWithPath: reference))
        try manager.saveClonedVoice(voice, to: URL(fileURLWithPath: output))
    }

    func synthesize(text: String, embeddings: String?, builtin: String?, language: String,
                    int8: Bool, output: String) async throws {
        await acquire()
        defer { release() }
        // Prefer full precision for voice fidelity; precision is not a user knob.
        let manager = try manager(language: language, int8: false)
        if !(await manager.isAvailable) { try await manager.initialize() }
        var samples: [Float] = []
        if let embeddings {
            let voice = try manager.loadClonedVoice(from: URL(fileURLWithPath: embeddings))
            let stream = try await manager.synthesizeStreaming(text: text, voiceData: voice)
            for try await frame in stream { samples.append(contentsOf: frame.samples) }
        } else {
            let stream = try await manager.synthesizeStreaming(text: text, voice: builtin)
            for try await frame in stream { samples.append(contentsOf: frame.samples) }
        }
        guard !samples.isEmpty, samples.allSatisfy({ $0.isFinite }) else {
            throw PocketTTSError.processingFailed("Model returned empty or non-finite audio")
        }
        // Preserve native gain below -1 dBFS. Attenuate overshoots before PCM
        // quantization, never amplify quiet/noisy output to full scale.
        let peak = samples.reduce(Float(0)) { max($0, abs($1)) }
        let gain: Float = peak > 0.89 ? 0.89 / peak : 1
        let leveled = samples.map { max(-0.89, min(0.89, $0 * gain)) }
        let data = try AudioWAV.data(from: leveled, sampleRate: 24000, normalize: false)
        // Foundation's atomic option writes a sibling temporary file then renames.
        try data.write(to: URL(fileURLWithPath: output), options: .atomic)
    }
}
#endif

private func ttsRequired(_ pointer: UnsafePointer<CChar>?, _ label: String) throws -> String {
    let value = pointer.map { String(cString: $0) } ?? ""
    guard !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
        throw NSError(domain: "UltraVox.VoiceStudio", code: 1,
                      userInfo: [NSLocalizedDescriptionKey: "\(label) must not be empty"])
    }
    return value
}

private func ttsBoundary(_ errorOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?,
                         operation: () throws -> Void) -> Int32 {
    errorOut?.pointee = nil
    do { try operation(); return 0 }
    catch {
        let message = "PocketTTS: \(error.localizedDescription) (\(error))"
        errorOut?.pointee = message.duplicateAsCChar()
        return 1
    }
}

@_cdecl("ultravox_macos_bridge_tts_clone_voice")
public func ultravox_macos_bridge_tts_clone_voice(
    _ refWavPath: UnsafePointer<CChar>?, _ outEmbeddingsPath: UnsafePointer<CChar>?,
    _ errorOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?
) -> Int32 {
    ttsBoundary(errorOut) {
        let reference = try ttsRequired(refWavPath, "Reference audio path")
        let output = try ttsRequired(outEmbeddingsPath, "Embeddings output path")
#if canImport(FluidAudio)
        try runAsyncAndBlock { try await PocketTtsEngine.shared.clone(reference: reference, output: output) }
#else
        throw NSError(domain: "FluidAudio unavailable", code: 1)
#endif
    }
}

@_cdecl("ultravox_macos_bridge_tts_prepare_models")
public func ultravox_macos_bridge_tts_prepare_models(
    _ language: UnsafePointer<CChar>?, _ useInt8: Int32,
    _ errorOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?
) -> Int32 {
    ttsBoundary(errorOut) {
        let language = try ttsRequired(language, "Language")
#if canImport(FluidAudio)
        try runAsyncAndBlock { try await PocketTtsEngine.shared.prepare(language: language, int8: useInt8 != 0) }
#else
        throw NSError(domain: "FluidAudio unavailable", code: 1)
#endif
    }
}

@_cdecl("ultravox_macos_bridge_tts_synthesize")
public func ultravox_macos_bridge_tts_synthesize(
    _ text: UnsafePointer<CChar>?, _ embeddingsPath: UnsafePointer<CChar>?,
    _ builtinVoice: UnsafePointer<CChar>?, _ language: UnsafePointer<CChar>?,
    _ useInt8: Int32, _ outWavPath: UnsafePointer<CChar>?,
    _ errorOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?
) -> Int32 {
    ttsBoundary(errorOut) {
        let text = try ttsRequired(text, "Text")
        let language = try ttsRequired(language, "Language")
        let output = try ttsRequired(outWavPath, "WAV output path")
        let embeddings = try embeddingsPath.map { try ttsRequired($0, "Embeddings path") }
        let builtin = try builtinVoice.map { try ttsRequired($0, "Builtin voice") }
#if canImport(FluidAudio)
        try runAsyncAndBlock {
            try await PocketTtsEngine.shared.synthesize(text: text, embeddings: embeddings,
                builtin: builtin, language: language, int8: useInt8 != 0, output: output)
        }
#else
        throw NSError(domain: "FluidAudio unavailable", code: 1)
#endif
    }
}
