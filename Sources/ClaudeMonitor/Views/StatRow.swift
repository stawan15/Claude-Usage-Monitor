import SwiftUI

/// One `label ········ value  detail` line in monospace.
struct StatRow: View {
    let label: String
    let value: String
    var detail: String?
    var emphasized = false

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(label)
                .foregroundStyle(emphasized ? Theme.text : Theme.dim)
                .lineLimit(1)
                .truncationMode(.middle)
            Spacer(minLength: 8)
            Text(value)
                .foregroundStyle(emphasized ? Theme.claude : Theme.text)
                .fontWeight(emphasized ? .semibold : .regular)
            if let detail {
                Text(detail)
                    .foregroundStyle(Theme.dim)
                    .frame(width: 62, alignment: .trailing)
            }
        }
    }
}
