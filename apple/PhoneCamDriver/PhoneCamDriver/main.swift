import CoreMediaIO
import Foundation

let configuration = CameraExtensionConfiguration.loadFromMainBundle()
let providerSource = CameraExtensionProviderSource(configuration: configuration, clientQueue: nil)

CMIOExtensionProvider.startService(provider: providerSource.provider)
CFRunLoopRun()
