import AVFoundation
import Foundation
import FoundationModels
import Speech

// One request per process keeps cancellation and wallet/session isolation explicit.
struct Request: Decodable {
    let operation: String
    let system: String?
    let prompt: String?
    let audio_path: String?
    let locale: String?
}
enum WorkerError: String, Error {
    case modelUnavailable = "model_unavailable"
    case speechModelMissing = "speech_model_missing"
    case unsupportedLanguage = "unsupported_language"
    case unsupportedAudio = "unsupported_audio"
    case requestRejected = "request_rejected"
    case invalidRequest = "invalid_request"
}
func emit(_ event: [String: Any]) {
    guard let data = try? JSONSerialization.data(withJSONObject: event) else { return }
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data([10]))
}

@main struct InvisibleAppleAI {
    static func main() async {
        do {
            let data = FileHandle.standardInput.readDataToEndOfFile()
            guard data.count <= 1_000_000 else { throw WorkerError.invalidRequest }
            let request = try JSONDecoder().decode(Request.self, from: data)
            switch request.operation {
            case "generate":
                // Explicit on-device model; never PrivateCloudComputeLanguageModel.
                let model = SystemLanguageModel.default
                guard case .available = model.availability else { throw WorkerError.modelUnavailable }
                guard let prompt = request.prompt, !prompt.isEmpty else { throw WorkerError.invalidRequest }
                let session = LanguageModelSession(model: model, instructions: request.system ?? "Answer using the supplied context.")
                var previous = ""
                do {
                    for try await snapshot in session.streamResponse(to: prompt) {
                        guard snapshot.content.hasPrefix(previous) else { throw WorkerError.requestRejected }
                        let delta = String(snapshot.content.dropFirst(previous.count))
                        if !delta.isEmpty { emit(["delta": delta]) }
                        previous = snapshot.content
                    }
                } catch { throw WorkerError.requestRejected }
                emit(["done": true])
            case "transcribe", "prepare-speech":
                let locale = Locale(identifier: request.locale ?? "en-US")
                guard let supported = await SpeechTranscriber.supportedLocale(equivalentTo: locale) else {
                    throw WorkerError.unsupportedLanguage
                }
                let transcriber = SpeechTranscriber(locale: supported, transcriptionOptions: [], reportingOptions: [], attributeOptions: [.audioTimeRange])
                let installed = await SpeechTranscriber.installedLocales
                if !installed.contains(supported) {
                    // Asset downloads are an explicit backend setup operation only.
                    guard request.operation == "prepare-speech" else { throw WorkerError.speechModelMissing }
                    if let assets = try await AssetInventory.assetInstallationRequest(supporting: [transcriber]) {
                        try await assets.downloadAndInstall()
                    }
                }
                if request.operation == "prepare-speech" {
                    emit(["delta": "Speech language is installed."])
                    emit(["done": true])
                    return
                }
                guard let path = request.audio_path else { throw WorkerError.invalidRequest }
                let file: AVAudioFile
                do { file = try AVAudioFile(forReading: URL(fileURLWithPath: path)) }
                catch { throw WorkerError.unsupportedAudio }
                let analyzer = SpeechAnalyzer(modules: [transcriber])
                let results = Task {
                    for try await result in transcriber.results {
                        emit(["delta": String(result.text.characters)])
                    }
                }
                do {
                    if let last = try await analyzer.analyzeSequence(from: file) {
                        try await analyzer.finalizeAndFinish(through: last)
                    } else {
                        await analyzer.cancelAndFinishNow()
                    }
                    try await results.value
                } catch {
                    results.cancel()
                    await analyzer.cancelAndFinishNow()
                    throw error
                }
                emit(["done": true])
            default: throw WorkerError.invalidRequest
            }
        } catch let error as WorkerError {
            emit(["error": error.rawValue])
        } catch {
            // Do not print prompts, file paths or framework diagnostics to the browser.
            emit(["error": "worker_failed"])
        }
    }
}
