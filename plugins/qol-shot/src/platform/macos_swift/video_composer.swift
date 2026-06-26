struct SegmentSpec {
    let offsetX: CGFloat
    let offsetY: CGFloat
    let destSize: CGSize
    let url: URL
}

struct LoadedSegment {
    let spec: SegmentSpec
    let asset: AVURLAsset
    let videoTrack: AVAssetTrack
    let naturalSize: CGSize
}

func fail(_ message: String) -> Never {
    fputs("\(message)\n", stderr)
    exit(1)
}

func usage() -> Never {
    fputs(
        "usage: video-composer <canvas-w> <canvas-h> <output-mov>"
            + " <offset-x> <offset-y> <dest-w> <dest-h> <input-mov>...\n",
        stderr
    )
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

func evenRendered(_ value: CGFloat) -> CGFloat {
    let rounded = Int(value.rounded())
    return CGFloat(max(rounded - (rounded % 2), 2))
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

func parseArguments() -> (canvas: CGSize, output: URL, specs: [SegmentSpec]) {
    let args = Array(CommandLine.arguments.dropFirst())
    guard args.count >= 8 && (args.count - 3) % 5 == 0 else {
        usage()
    }

    let canvas = CGSize(
        width: parseCGFloat(args[0], "canvas width"),
        height: parseCGFloat(args[1], "canvas height")
    )
    guard canvas.width > 0 && canvas.height > 0 else {
        fail("canvas size must be positive")
    }

    let output = URL(fileURLWithPath: args[2])
    var specs: [SegmentSpec] = []
    var index = 3
    while index < args.count {
        let destSize = CGSize(
            width: parseCGFloat(args[index + 2], "dest width"),
            height: parseCGFloat(args[index + 3], "dest height")
        )
        guard destSize.width > 0 && destSize.height > 0 else {
            fail("segment destination size must be positive")
        }
        specs.append(SegmentSpec(
            offsetX: parseCGFloat(args[index], "offset x"),
            offsetY: parseCGFloat(args[index + 1], "offset y"),
            destSize: destSize,
            url: URL(fileURLWithPath: args[index + 4])
        ))
        index += 5
    }

    return (canvas, output, specs)
}

func loadSegments(_ specs: [SegmentSpec]) -> [LoadedSegment] {
    specs.map { spec in
        let asset = AVURLAsset(url: spec.url)
        guard let videoTrack = asset.tracks(withMediaType: .video).first else {
            fail("missing video track: \(spec.url.path)")
        }
        let naturalSize = videoTrack.naturalSize
        guard naturalSize.width > 0 && naturalSize.height > 0 else {
            fail("segment has no video frames: \(spec.url.path)")
        }
        return LoadedSegment(
            spec: spec,
            asset: asset,
            videoTrack: videoTrack,
            naturalSize: naturalSize
        )
    }
}

func commonDuration(_ loaded: [LoadedSegment]) -> CMTime {
    var duration = loaded[0].asset.duration
    for segment in loaded.dropFirst() {
        duration = minTime(duration, segment.asset.duration)
    }
    guard positiveDuration(duration) else {
        fail("input segments have no positive duration")
    }
    return duration
}

func outputScale(_ loaded: [LoadedSegment]) -> CGFloat {
    loaded.reduce(1) { scale, segment in
        let scaleX = segment.naturalSize.width / segment.spec.destSize.width
        let scaleY = segment.naturalSize.height / segment.spec.destSize.height
        return max(scale, max(scaleX, scaleY))
    }
}

func placementTransform(for segment: LoadedSegment, scale: CGFloat) -> CGAffineTransform {
    let spec = segment.spec
    let resample = CGAffineTransform(
        scaleX: scale * spec.destSize.width / segment.naturalSize.width,
        y: scale * spec.destSize.height / segment.naturalSize.height
    )
    let placement = CGAffineTransform(
        translationX: scale * spec.offsetX,
        y: scale * spec.offsetY
    )
    return normalizedTransform(for: segment.videoTrack)
        .concatenating(resample)
        .concatenating(placement)
}

func makeComposition(
    from loaded: [LoadedSegment],
    duration: CMTime
) -> (AVMutableComposition, [AVMutableCompositionTrack]) {
    let composition = AVMutableComposition()
    let timeRange = CMTimeRange(start: .zero, duration: duration)
    var videoTracks: [AVMutableCompositionTrack] = []

    for segment in loaded {
        guard let track = composition.addMutableTrack(
            withMediaType: .video,
            preferredTrackID: kCMPersistentTrackID_Invalid
        ) else {
            fail("failed to create composition video track")
        }
        do {
            try track.insertTimeRange(timeRange, of: segment.videoTrack, at: .zero)
        } catch {
            fail("failed to insert video track \(segment.spec.url.path): \(error)")
        }
        videoTracks.append(track)
    }

    insertAudio(from: loaded[0], into: composition, duration: duration)
    return (composition, videoTracks)
}

func insertAudio(from segment: LoadedSegment, into composition: AVMutableComposition, duration: CMTime) {
    guard let audioTrack = segment.asset.tracks(withMediaType: .audio).first,
          let compositionAudio = composition.addMutableTrack(
              withMediaType: .audio,
              preferredTrackID: kCMPersistentTrackID_Invalid
          )
    else {
        return
    }
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

func makeVideoComposition(
    loaded: [LoadedSegment],
    tracks: [AVMutableCompositionTrack],
    canvas: CGSize,
    scale: CGFloat,
    duration: CMTime
) -> AVMutableVideoComposition {
    let layerInstructions = zip(loaded, tracks).map { segment, track -> AVMutableVideoCompositionLayerInstruction in
        let instruction = AVMutableVideoCompositionLayerInstruction(assetTrack: track)
        instruction.setTransform(placementTransform(for: segment, scale: scale), at: .zero)
        return instruction
    }

    let instruction = AVMutableVideoCompositionInstruction()
    instruction.timeRange = CMTimeRange(start: .zero, duration: duration)
    instruction.layerInstructions = layerInstructions
    instruction.backgroundColor = CGColor(gray: 0, alpha: 1)

    let renderSize = CGSize(
        width: evenRendered(canvas.width * scale),
        height: evenRendered(canvas.height * scale)
    )
    fputs(
        "video-composer: segments=\(loaded.count) canvas=\(Int(canvas.width))x\(Int(canvas.height))"
            + " scale=\(scale) render=\(Int(renderSize.width))x\(Int(renderSize.height))\n",
        stderr
    )

    let videoComposition = AVMutableVideoComposition()
    videoComposition.renderSize = renderSize
    videoComposition.frameDuration = CMTime(value: 1, timescale: 60)
    videoComposition.instructions = [instruction]
    return videoComposition
}

func export(_ composition: AVMutableComposition, using videoComposition: AVMutableVideoComposition, to outputURL: URL) {
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
}

let (canvas, outputURL, specs) = parseArguments()
let loaded = loadSegments(specs)
let duration = commonDuration(loaded)
let scale = outputScale(loaded)
let (composition, videoTracks) = makeComposition(from: loaded, duration: duration)
let videoComposition = makeVideoComposition(
    loaded: loaded,
    tracks: videoTracks,
    canvas: canvas,
    scale: scale,
    duration: duration
)
export(composition, using: videoComposition, to: outputURL)
