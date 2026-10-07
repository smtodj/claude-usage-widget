// Asks WidgetKit to redraw the Claude Usage widget. The app runs this after
// writing new numbers to the widget's container. It lives in the app's
// Contents/MacOS, so WidgetKit sees it as the widget's containing app.

import Foundation
import WidgetKit

WidgetCenter.shared.reloadTimelines(ofKind: "ClaudeUsageWidget")
// The request is delivered asynchronously; give it a moment before exiting.
RunLoop.main.run(until: Date().addingTimeInterval(0.5))
