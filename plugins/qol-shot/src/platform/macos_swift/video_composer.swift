struct SegmentInput {
    let offsetX: CGFloat
    let offsetY: CGFloat
    let url: URL
    let asset: AVURLAsset
    let videoTrack: AVAssetTrack
}

func fail(_ message: String) -> Never {
    fputs("\(message)\n", stderr)
    exit(1)
}

func usage() -> Never {
    fputs("usage: video-composer <canvas-w> <canvas-h> <output-mov> <offset-x> <offset-y> <input-mov>...\n", stderr)
    exit(64)
}

func parseCGFloat(_ raw: String, _ label: String) -> CGFloat {
    guard let value = Double(raw) else {
        fail("invalid \(label): \(raw)")
    }
    return CGFloat(value)
}

func minTime(_ left: CMTime, _ right: CMTime) -> CMTime {
    CMTimeCompare(left, right) <= 0 ? left : right
}

func positiveDuration(_ duration: CMTime) -> Bool {
    duration.isValid && duration.isNumeric && CMTimeCompare(duration, .zero) > 0
}

func normalizedTransform(for track: AVAssetTrack) -> CGAffineTransform {
    var transform = track.preferredTransform
    let transformed = track.naturalSize.applying(transform)

    if transformed.width < 0 {
        transform = transform.translatedBy(x: abs(transformed.width), y: 0)
    }

    if transformed.height < 0 {
        transform = transform.translatedBy(x: 0, y: abs(transformed.height))
    }

    return transform
}

let args = Array(CommandLine.arguments.dropFirst())
guard args.count >= 6 && (args.count - 3) % 3 == 0 else {
    usage()
}

let canvasWidth = parseCGFloat(args[0], "canvas width")
let canvasHeight = parseCGFloat(args[1], "canvas height")
guard canvasWidth > 0 && canvasHeight > 0 else {
    fail("canvas size must be positive")
}

let outputURL = URL(fileURLWithPath: args[2])
var inputs: [SegmentInput] = []
var index = 3
while index < args.count {
    let offsetX = parseCGFloat(args[index], "offset x")
    let offsetY = parseCGFloat(args[index + 1], "offset y")
    let url = URL(fileURLWithPath: args[index + 2])
    let asset = AVURLAsset(url: url)

    guard let videoTrack = asset.tracks(withMediaType: .video).first else {
        fail("missing video track: \(url.path)")
    }

    inputs.append(SegmentInput(
        offsetX: offsetX,
        offsetY: offsetY,
        url: url,
        asset: asset,
        videoTrack: videoTrack
    ))
    index += 3
}

guard !inputs.isEmpty else {
    usage()
}

var duration = inputs[0].asset.duration
for input in inputs.dropFirst() {
    duration = minTime(duration, input.asset.duration)
}
guard positiveDuration(duration) else {
    fail("input segments have no positive duration")
}

let composition = AVMutableComposition()
var layerInstructions: [AVMutableVideoCompositionLayerInstruction] = []
let timeRange = CMTimeRange(start: .zero, duration: duration)

for input in inputs {
    guard let track = composition.addMutableTrack(
        withMediaType: .video,
        preferredTrackID: kCMPersistentTrackID_Invalid
    ) else {
        fail("failed to create composition video track")
    }

    do {
        try track.insertTimeRange(timeRange, of: input.videoTrack, at: .zero)
    } catch {
        fail("failed to insert video track \(input.url.path): \(error)")
    }

    let instruction = AVMutableVideoCompositionLayerInstruction(assetTrack: track)
    let transform = normalizedTransform(for: input.videoTrack)
        .concatenating(CGAffineTransform(translationX: input.offsetX, y: input.offsetY))
    instruction.setTransform(transform, at: .zero)
    layerInstructions.append(instruction)
}

if let audioTrack = inputs[0].asset.tracks(withMediaType: .audio).first,
   let compositionAudio = composition.addMutableTrack(
        withMediaType: .audio,
        preferredTrackID: kCMPersistentTrackID_Invalid
   ) {
    do {
        try compositionAudio.insertTimeRange(
            CMTimeRange(start: .zero, duration: minTime(duration, audioTrack.timeRange.duration)),
            of: audioTrack,
            at: .zero
        )
    } catch {
        fputs("warning: failed to insert audio track: \(error)\n", stderr)
    }
}

let instruction = AVMutableVideoCompositionInstruction()
instruction.timeRange = timeRange
instruction.layerInstructions = layerInstructions
instruction.backgroundColor = CGColor(gray: 0, alpha: 1)

let videoComposition = AVMutableVideoComposition()
videoComposition.renderSize = CGSize(width: canvasWidth, height: canvasHeight)
videoComposition.frameDuration = CMTime(value: 1, timescale: 60)
videoComposition.instructions = [instruction]

if FileManager.default.fileExists(atPath: outputURL.path) {
    do {
        try FileManager.default.removeItem(at: outputURL)
    } catch {
        fail("failed to replace output file: \(error)")
    }
}

guard let exporter = AVAssetExportSession(
    asset: composition,
    presetName: AVAssetExportPresetHighestQuality
) else {
    fail("failed to create AVAssetExportSession")
}

exporter.outputURL = outputURL
exporter.outputFileType = .mov
exporter.videoComposition = videoComposition

let semaphore = DispatchSemaphore(value: 0)
exporter.exportAsynchronously {
    semaphore.signal()
}
semaphore.wait()

guard exporter.status == .completed else {
    let detail = exporter.error.map { ": \($0)" } ?? ""
    fail("native video composition failed with status \(exporter.status.rawValue)\(detail)")
}
