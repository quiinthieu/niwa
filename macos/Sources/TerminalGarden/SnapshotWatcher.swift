import Darwin
import Foundation

/// Zero-polling `DispatchSourceFileSystemObject` (`kqueue` `EVFILT_VNODE`) watcher for
/// `snapshot.json` (`PLAN.md` §0.6, §7 & §10 T14; `docs/platform/macos-menubar.md` §3).
///
/// Design invariants:
/// - Never creates directories or writes files (`PLAN.md` §0.6: only `tg-store` writes to disk).
///   If `dataDirectory` does not exist yet on first launch, watches the nearest existing ancestor
///   directory with `O_EVTONLY` and promotes the watch automatically once `tg-store` creates it.
/// - Handles `tg_store::snapshot::write_snapshot`'s atomic temp-file `rename(2)` replacement by
///   watching both `dataDirectory` and `snapshot.json` and re-arming the file descriptor whenever
///   the old inode receives `.delete` or `.rename`.
public final class SnapshotWatcher: @unchecked Sendable {
    private let paths: NiwaDataPaths
    private let queue: DispatchQueue
    private let onChange: @Sendable () -> Void

    private let lock = NSLock()
    private var directorySource: DispatchSourceFileSystemObject?
    private var watchedDirectoryPath: String?
    private var fileSource: DispatchSourceFileSystemObject?
    private var watchedFileInode: UInt64?
    private var isRunning = false

    public init(
        paths: NiwaDataPaths,
        queue: DispatchQueue = DispatchQueue(
            label: "\(ProductMetadata.current.bundleId).snapshot-watcher",
            qos: .utility
        ),
        onChange: @escaping @Sendable () -> Void
    ) {
        self.paths = paths
        self.queue = queue
        self.onChange = onChange
    }

    deinit {
        stop()
    }

    public func start() {
        lock.lock()
        if isRunning {
            lock.unlock()
            return
        }
        isRunning = true
        refreshSourcesLocked()
        lock.unlock()
    }

    public func stop() {
        lock.lock()
        isRunning = false
        let oldDir = directorySource
        let oldFile = fileSource
        directorySource = nil
        watchedDirectoryPath = nil
        fileSource = nil
        watchedFileInode = nil
        lock.unlock()

        oldDir?.cancel()
        oldFile?.cancel()
    }

    private func handleFileSystemEvent(forceReopenFile: Bool) {
        var shouldNotify = false
        lock.lock()
        if isRunning {
            if forceReopenFile {
                let stale = fileSource
                fileSource = nil
                watchedFileInode = nil
                stale?.cancel()
            }
            refreshSourcesLocked()
            shouldNotify = true
        }
        lock.unlock()

        if shouldNotify {
            onChange()
        }
    }

    private func refreshSourcesLocked() {
        guard isRunning else { return }

        if let targetDir = nearestExistingDirectoryPath(startingAt: paths.dataDirectory) {
            if watchedDirectoryPath != targetDir || directorySource == nil {
                let staleDir = directorySource
                directorySource = nil
                watchedDirectoryPath = nil
                staleDir?.cancel()

                if let newDirSource = makeSource(
                    path: targetDir,
                    mask: [.write, .rename, .delete, .extend],
                    isFileTarget: false
                ) {
                    directorySource = newDirSource
                    watchedDirectoryPath = targetDir
                }
            }
        }

        let snapshotPath = paths.snapshotFile.path
        if let attrs = try? FileManager.default.attributesOfItem(atPath: snapshotPath),
           let currentInode = (attrs[.systemFileNumber] as? NSNumber)?.uint64Value
        {
            if fileSource == nil || watchedFileInode != currentInode {
                let staleFile = fileSource
                fileSource = nil
                watchedFileInode = nil
                staleFile?.cancel()

                if let newFileSource = makeSource(
                    path: snapshotPath,
                    mask: [.write, .extend, .attrib, .delete, .rename],
                    isFileTarget: true
                ) {
                    fileSource = newFileSource
                    watchedFileInode = currentInode
                }
            }
        } else if let staleFile = fileSource {
            fileSource = nil
            watchedFileInode = nil
            staleFile.cancel()
        }
    }

    private func nearestExistingDirectoryPath(startingAt directoryURL: URL) -> String? {
        var candidate = directoryURL.standardizedFileURL
        let fm = FileManager.default
        while true {
            var isDir: ObjCBool = false
            if fm.fileExists(atPath: candidate.path, isDirectory: &isDir), isDir.boolValue {
                return candidate.path
            }
            let parent = candidate.deletingLastPathComponent()
            if parent.path == candidate.path {
                return nil
            }
            candidate = parent
        }
    }

    private func makeSource(
        path: String,
        mask: DispatchSource.FileSystemEvent,
        isFileTarget: Bool
    ) -> DispatchSourceFileSystemObject? {
        let fd = Darwin.open(path, O_EVTONLY)
        guard fd >= 0 else {
            return nil
        }

        let source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: fd,
            eventMask: mask,
            queue: queue
        )
        source.setEventHandler { [weak self, weak source] in
            guard let self else { return }
            let flags = source?.data ?? []
            let replaced = isFileTarget && (flags.contains(.delete) || flags.contains(.rename))
            self.handleFileSystemEvent(forceReopenFile: replaced)
        }
        source.setCancelHandler {
            Darwin.close(fd)
        }
        source.resume()
        return source
    }
}
