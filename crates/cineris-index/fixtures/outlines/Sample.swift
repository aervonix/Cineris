import Foundation

/// A point in space.
public struct Point: Equatable {
    public var x: Double
    public var y: Double

    /// Makes a point.
    public init(x: Double, y: Double) {
        self.x = x
        self.y = y
    }

    public static func == (lhs: Point, rhs: Point) -> Bool {
        return lhs.x == rhs.x
    }

    mutating func move(by delta: Double) {
        let text = "func fake() {"
        x += delta
    }
}

protocol Shape {
    func area() -> Double
    var name: String { get }
}

final class Circle: Shape {
    let radius: Double

    init?(radius: Double) {
        guard radius > 0 else { return nil }
        self.radius = radius
    }

    deinit {}

    class func unit() -> Circle { Circle(radius: 1)! }

    func area() -> Double { .pi * radius * radius }

    var name: String { "circle" }
}

extension Point: CustomStringConvertible {
    public var description: String { "(\(x), \(y))" }

    func scaled(_ factor: Double) -> Point {
        Point(x: x * factor, y: y * factor)
    }
}

enum Direction {
    case north, south

    func opposite() -> Direction { self == .north ? .south : .north }
}

@MainActor
actor Counter {
    private var value = 0
    func increment() { value += 1 }
}

typealias Handler = (Point) -> Void

let multiline = """
func notReal() {
"""
