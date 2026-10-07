// Desktop / Notification Center widget showing how much Claude usage is left.
//
// The menu bar app writes the latest reading to `usage.json` in this
// extension's sandbox container (its home directory) and asks WidgetKit to
// reload. The widget only reads that file; it never talks to the network.

import SwiftUI
import WidgetKit

struct UsageWindow: Decodable, Hashable {
    let key: String
    let label: String
    let remainingPercent: Double
    /// Unix seconds.
    let resetsAt: Double?

    enum CodingKeys: String, CodingKey {
        case key, label
        case remainingPercent = "remaining_percent"
        case resetsAt = "resets_at"
    }
}

struct UsageData: Decodable {
    /// Unix seconds of the last check.
    let updatedAt: Double
    let windows: [UsageWindow]
    /// Set when the last check failed; `windows` is then the last good reading.
    let error: String?

    enum CodingKeys: String, CodingKey {
        case updatedAt = "updated_at"
        case windows, error
    }

    static func load() -> UsageData? {
        let url = URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent("usage.json")
        guard let data = try? Data(contentsOf: url) else { return nil }
        return try? JSONDecoder().decode(UsageData.self, from: data)
    }

    static let sample = UsageData(
        updatedAt: Date().timeIntervalSince1970,
        windows: [
            UsageWindow(key: "five_hour", label: "세션", remainingPercent: 82,
                        resetsAt: Date().addingTimeInterval(3 * 3600 + 41 * 60).timeIntervalSince1970),
            UsageWindow(key: "seven_day", label: "주간", remainingPercent: 71,
                        resetsAt: Date().addingTimeInterval(3 * 86400 + 2 * 3600).timeIntervalSince1970),
        ],
        error: nil
    )
}

struct UsageEntry: TimelineEntry {
    let date: Date
    let data: UsageData?
}

struct Provider: TimelineProvider {
    func placeholder(in context: Context) -> UsageEntry {
        UsageEntry(date: Date(), data: .sample)
    }

    func getSnapshot(in context: Context, completion: @escaping (UsageEntry) -> Void) {
        let data = context.isPreview ? (UsageData.load() ?? .sample) : UsageData.load()
        completion(UsageEntry(date: Date(), data: data))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<UsageEntry>) -> Void) {
        let data = UsageData.load()
        // One entry per minute so the reset countdown keeps moving between
        // the app's updates; the app also asks for a reload when numbers change.
        let now = Date()
        let start = Calendar.current.dateInterval(of: .minute, for: now)?.start ?? now
        let entries = (0..<60).map { i in
            UsageEntry(date: i == 0 ? now : start.addingTimeInterval(Double(i) * 60), data: data)
        }
        completion(Timeline(entries: entries, policy: .after(now.addingTimeInterval(15 * 60))))
    }
}

// MARK: - Views

private func shortLabel(_ w: UsageWindow) -> String {
    switch w.key {
    case "five_hour": return "세션"
    case "seven_day": return "주간"
    case "seven_day_opus": return "Opus"
    case "seven_day_sonnet": return "Sonnet"
    default: return w.label
    }
}

private func barColor(_ remaining: Double) -> Color {
    if remaining <= 15 { return .red }
    if remaining <= 40 { return .orange }
    return .green
}

private func resetText(_ resetsAt: Double?, now: Date) -> String {
    guard let resetsAt else { return "초기화 시각 미정" }
    let mins = max(0, Int((resetsAt - now.timeIntervalSince1970) / 60))
    let d = mins / 1440, h = (mins % 1440) / 60, m = mins % 60
    let text = d > 0 ? "\(d)일 \(h)시간" : h > 0 ? "\(h)시간 \(m)분" : "\(m)분"
    return "\(text) 후 리셋"
}

struct WindowRow: View {
    let window: UsageWindow
    let now: Date
    let compact: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: compact ? 3 : 4) {
            HStack(alignment: .firstTextBaseline) {
                Text(shortLabel(window))
                    .font(.system(size: compact ? 11 : 12, weight: .semibold))
                Spacer(minLength: 4)
                Text("\(Int(window.remainingPercent.rounded()))% 남음")
                    .font(.system(size: compact ? 11 : 12, weight: .semibold))
                    .monospacedDigit()
            }
            GeometryReader { geo in
                ZStack(alignment: .leading) {
                    Capsule().fill(Color.primary.opacity(0.15))
                    Capsule()
                        .fill(barColor(window.remainingPercent))
                        .frame(width: geo.size.width * max(0, min(1, window.remainingPercent / 100)))
                }
            }
            .frame(height: compact ? 5 : 6)
            Label(resetText(window.resetsAt, now: now), systemImage: "arrow.clockwise")
                .font(.system(size: compact ? 9 : 10))
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
    }
}

struct UsageWidgetView: View {
    @Environment(\.widgetFamily) private var family
    let entry: UsageEntry

    private var compact: Bool { family == .systemSmall }

    private func shownWindows(_ data: UsageData) -> [UsageWindow] {
        let main = ["five_hour", "seven_day"]
        let first = data.windows.filter { main.contains($0.key) }
        guard !compact else { return first }
        let rest = data.windows.filter { !main.contains($0.key) }
        return Array((first + rest).prefix(3))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: compact ? 8 : 10) {
            HStack(spacing: 5) {
                Image(systemName: "sparkle")
                    .foregroundStyle(Color(red: 0.85, green: 0.47, blue: 0.34))
                Text("Claude")
                    .font(.system(size: 13, weight: .semibold))
                Spacer(minLength: 0)
                if let data = entry.data, data.error != nil || isStale(data) {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .font(.system(size: 10))
                        .foregroundStyle(.orange)
                }
            }
            if let data = entry.data, !data.windows.isEmpty {
                ForEach(shownWindows(data), id: \.self) { w in
                    WindowRow(window: w, now: entry.date, compact: compact)
                }
                Spacer(minLength: 0)
                if isStale(data) {
                    Text("\(minutesAgo(data))분 전 값 · 앱이 켜져 있는지 확인해 주세요")
                        .font(.system(size: 9))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
            } else {
                Spacer(minLength: 0)
                Text(entry.data?.error ?? "메뉴 막대의 Claude Usage 앱을 실행하고 로그인해 주세요.")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                Spacer(minLength: 0)
            }
        }
        .containerBackground(.fill.tertiary, for: .widget)
    }

    private func minutesAgo(_ data: UsageData) -> Int {
        max(0, Int((entry.date.timeIntervalSince1970 - data.updatedAt) / 60))
    }

    private func isStale(_ data: UsageData) -> Bool {
        minutesAgo(data) >= 15
    }
}

struct UsageWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "ClaudeUsageWidget", provider: Provider()) { entry in
            UsageWidgetView(entry: entry)
        }
        .configurationDisplayName("Claude 사용량")
        .description("남은 사용량과 초기화까지 남은 시간을 보여줘요.")
        .supportedFamilies([.systemSmall, .systemMedium])
    }
}

@main
struct UsageWidgetBundle: WidgetBundle {
    var body: some Widget {
        UsageWidget()
    }
}
