// Public macOS image/Vision APIs verify rendered pixels, not a DOM simulation.
import AppKit
import Vision

let path = CommandLine.arguments[1]
guard let data = NSData(contentsOfFile: path),
      let bitmap = NSBitmapImageRep(data: data as Data),
      let image = bitmap.cgImage else { fatalError("Cannot decode WebView screenshot") }
let request = VNRecognizeTextRequest()
request.recognitionLevel = .accurate
request.recognitionLanguages = ["en-US"]
request.usesLanguageCorrection = false
try VNImageRequestHandler(cgImage: image).perform([request])
let width = bitmap.pixelsWide
let height = bitmap.pixelsHigh
let text = (request.results ?? []).compactMap { item -> [String: Any]? in
    guard let candidate = item.topCandidates(1).first else { return nil }
    let box = item.boundingBox
    return ["text": candidate.string, "confidence": candidate.confidence,
            "rectangle": ["x": box.minX * Double(width),
                          "y": (1 - box.maxY) * Double(height),
                          "width": box.width * Double(width),
                          "height": box.height * Double(height)]]
}
var count = 0, minX = width, minY = height, maxX = 0, maxY = 0
for y in 0..<height {
    for x in 0..<width {
        // Inspect encoded screenshot samples. Converting a calibrated bitmap to
        // the active display profile changes pure red (for example to G=0.149).
        guard let color = bitmap.colorAt(x: x, y: y) else { continue }
        if color.redComponent > 0.95 && color.greenComponent < 0.05 && color.blueComponent < 0.05 {
            count += 1; minX = min(minX, x); maxX = max(maxX, x)
            minY = min(minY, y); maxY = max(maxY, y)
        }
    }
}
let result: [String: Any] = ["width": width, "height": height, "recognizedText": text,
    "redPixelCount": count, "redRectangle": ["x": minX, "y": minY,
    "width": count == 0 ? 0 : maxX - minX + 1, "height": count == 0 ? 0 : maxY - minY + 1]]
let json = try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys])
print(String(decoding: json, as: UTF8.self))
