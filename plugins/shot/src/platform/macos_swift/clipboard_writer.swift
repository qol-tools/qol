let arguments = CommandLine.arguments
guard arguments.count == 2 else {
    fputs("usage: clipboard-writer <png-path>\n", stderr)
    exit(64)
}

let imageURL = URL(fileURLWithPath: arguments[1])
guard let data = try? Data(contentsOf: imageURL) else {
    fputs("failed to read image data\n", stderr)
    exit(66)
}

guard NSBitmapImageRep(data: data) != nil else {
    fputs("failed to decode image data\n", stderr)
    exit(65)
}

let pasteboard = NSPasteboard.general
pasteboard.clearContents()
guard pasteboard.setData(data, forType: .png) else {
    fputs("failed to write image data to pasteboard\n", stderr)
    exit(1)
}
