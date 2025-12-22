import Cocoa
import SwiftUI

class AppDelegate: NSObject, NSApplicationDelegate {
    var statusItem: NSStatusItem!
    var loginWindow: NSWindow?
    var isLoggedIn = false

    func applicationDidFinishLaunching(_ aNotification: Notification) {
        // Create menubar item FIRST - this works!
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)

        if let button = statusItem.button {
            button.title = "E"
            button.font = NSFont.boldSystemFont(ofSize: 16)
            button.toolTip = "Entanglement - File Syncing"
        }

        updateMenu()

        print("✅ Entanglement menubar app started")
    }

    func updateMenu() {
        let menu = NSMenu()

        menu.addItem(NSMenuItem(title: "Status: Disconnected", action: nil, keyEquivalent: ""))
        menu.addItem(NSMenuItem.separator())
        menu.addItem(NSMenuItem(title: "Log In", action: #selector(showLogin), keyEquivalent: "l"))
        menu.addItem(NSMenuItem.separator())
        menu.addItem(NSMenuItem(title: "Quit Entanglement", action: #selector(quit), keyEquivalent: "q"))

        statusItem.menu = menu
    }

    @objc private func showLogin() {
        // Simple login window for now
        let alert = NSAlert()
        alert.messageText = "Entanglement Login"
        alert.informativeText = "Login functionality will be implemented here"
        alert.addButton(withTitle: "OK")
        alert.runModal()
    }

    @objc func quit() {
        NSApplication.shared.terminate(nil)
    }
}