// Asks WidgetKit to redraw a widget: `widget-reload <kind>`, or all of
// them without an argument. The app runs this after writing new numbers to
// the widgets' container. It lives in the app's Contents/MacOS, so
// WidgetKit sees it as the widgets' containing app.

import Foundation
import WidgetKit

if CommandLine.arguments.count > 1 {
    WidgetCenter.shared.reloadTimelines(ofKind: CommandLine.arguments[1])
} else {
    WidgetCenter.shared.reloadAllTimelines()
}
// The request is delivered asynchronously; give it a moment before exiting.
RunLoop.main.run(until: Date().addingTimeInterval(0.5))
