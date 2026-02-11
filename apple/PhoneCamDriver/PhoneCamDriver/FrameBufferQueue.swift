import CoreMedia
import Foundation

final class FrameBufferQueue {
    private let capacity: Int
    private let queue = DispatchQueue(
        label: "com.phonecam.driver.cameraextension.framebuffer",
        qos: .userInteractive
    )

    private var storage: [CMSampleBuffer?]
    private var headIndex = 0
    private var itemCount = 0

    init(capacity: Int) {
        let normalizedCapacity = max(1, capacity)
        self.capacity = normalizedCapacity
        self.storage = Array(repeating: nil, count: normalizedCapacity)
    }

    func enqueue(_ sampleBuffer: CMSampleBuffer) {
        queue.sync {
            if itemCount == capacity {
                storage[headIndex] = sampleBuffer
                headIndex = (headIndex + 1) % capacity
                return
            }

            let tailIndex = (headIndex + itemCount) % capacity
            storage[tailIndex] = sampleBuffer
            itemCount += 1
        }
    }

    func dequeue() -> CMSampleBuffer? {
        queue.sync {
            guard itemCount > 0 else {
                return nil
            }

            let sampleBuffer = storage[headIndex]
            storage[headIndex] = nil
            headIndex = (headIndex + 1) % capacity
            itemCount -= 1
            return sampleBuffer
        }
    }

    func removeAll() {
        queue.sync {
            storage = Array(repeating: nil, count: capacity)
            headIndex = 0
            itemCount = 0
        }
    }
}
