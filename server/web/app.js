/**
 * Entanglement Web File Manager
 * Lightweight client for the Entanglement file sync server
 */

// Dynamically determine API base URL:
// Uses the current host with API port 1975
const API_BASE = `${window.location.protocol}//${window.location.hostname}:1975`;

// Register service worker for PWA support
if ('serviceWorker' in navigator) {
    navigator.serviceWorker.register('/sw.js')
        .then(reg => console.log('Service Worker registered:', reg.scope))
        .catch(err => console.warn('Service Worker registration failed:', err));
}

// Audio file extensions (defined early for use in renderFileList)
const AUDIO_EXTENSIONS = ['mp3', 'wav', 'aac', 'flac', 'ogg', 'm4a', 'wma', 'aiff', 'aif', 'alac', 'opus'];

function isAudioFile(filename) {
    const ext = filename.split('.').pop().toLowerCase();
    return AUDIO_EXTENSIONS.includes(ext);
}

// Video file extensions
const VIDEO_EXTENSIONS = ['mp4', 'mov', 'webm', 'm4v', 'ogv', 'avi', 'mkv'];

function isVideoFile(filename) {
    const ext = filename.split('.').pop().toLowerCase();
    return VIDEO_EXTENSIONS.includes(ext);
}

// State
let state = {
    token: localStorage.getItem('entanglement_token'),
    username: localStorage.getItem('entanglement_username'),
    isAdmin: localStorage.getItem('entanglement_is_admin') === 'true',
    currentPath: '',
    serverName: 'Entanglement', // Default fallback
};

// SVG icons for context menu (defined early for use in renderFileList)
const ICON_NEW_FOLDER = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" fill="currentColor"><path d="M216,72H131.31L104,44.69A15.86,15.86,0,0,0,92.69,40H40A16,16,0,0,0,24,56V200.62A15.4,15.4,0,0,0,39.38,216H216.89A15.13,15.13,0,0,0,232,200.89V88A16,16,0,0,0,216,72ZM40,56H92.69l16,16H40ZM216,200H40V88H216Z"/></svg>';
const ICON_RENAME = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" fill="currentColor"><path d="M227.31,73.37,182.63,28.68a16,16,0,0,0-22.63,0L36.69,152A15.86,15.86,0,0,0,32,163.31V208a16,16,0,0,0,16,16H92.69A15.86,15.86,0,0,0,104,219.31L227.31,96a16,16,0,0,0,0-22.63ZM92.69,208H48V163.31l88-88L180.69,120ZM192,108.68,147.31,64l24-24L216,84.68Z"/></svg>';
const ICON_DELETE = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" fill="currentColor"><path d="M216,48H176V40a24,24,0,0,0-24-24H104A24,24,0,0,0,80,40v8H40a8,8,0,0,0,0,16h8V208a16,16,0,0,0,16,16H192a16,16,0,0,0,16-16V64h8a8,8,0,0,0,0-16ZM96,40a8,8,0,0,1,8-8h48a8,8,0,0,1,8,8v8H96Zm96,168H64V64H192ZM112,104v64a8,8,0,0,1-16,0V104a8,8,0,0,1,16,0Zm48,0v64a8,8,0,0,1-16,0V104a8,8,0,0,1,16,0Z"/></svg>';
const ICON_SHARE = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" fill="currentColor"><path d="M229.66,109.66l-48,48a8,8,0,0,1-11.32-11.32L204.69,112H165a88.1,88.1,0,0,0-85.23,66,8,8,0,0,1-15.5-4A104.12,104.12,0,0,1,165,96h39.71L170.34,61.66a8,8,0,0,1,11.32-11.32l48,48A8,8,0,0,1,229.66,109.66ZM192,208H40V88a8,8,0,0,0-16,0V216a8,8,0,0,0,8,8H192a8,8,0,0,0,0-16Z"/></svg>';
const ICON_DOWNLOAD = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" fill="currentColor"><path d="M224,144v64a8,8,0,0,1-8,8H40a8,8,0,0,1-8-8V144a8,8,0,0,1,16,0v56H208V144a8,8,0,0,1,16,0Zm-101.66,5.66a8,8,0,0,0,11.32,0l40-40a8,8,0,0,0-11.32-11.32L136,124.69V32a8,8,0,0,0-16,0v92.69L93.66,98.34a8,8,0,0,0-11.32,11.32Z"/></svg>';

// DOM Elements
const loginView = document.getElementById('login-view');
const browserView = document.getElementById('browser-view');
const usersView = document.getElementById('users-view');
const loginForm = document.getElementById('login-form');
const loginError = document.getElementById('login-error');
const serverStatus = document.getElementById('server-status');
const statusIndicator = document.querySelector('.status-indicator');
const userNameEl = document.getElementById('user-name');
const logoutBtn = document.getElementById('logout-btn');
const breadcrumb = document.getElementById('breadcrumb');
const fileList = document.getElementById('file-list');
const emptyState = document.getElementById('empty-state');
const loadingState = document.getElementById('loading-state');
const itemCount = document.getElementById('item-count');
const currentTime = document.getElementById('current-time');
const userMenuBtn = document.getElementById('user-menu-btn');
const userDropdown = document.getElementById('user-dropdown');
const previewModal = document.getElementById('preview-modal');
const previewContent = document.getElementById('preview-content');
const previewClose = document.getElementById('preview-close');
const previewBackdrop = document.querySelector('.preview-backdrop');
const contextMenu = document.getElementById('context-menu');
const fileBrowser = document.querySelector('.file-browser');

// =============================================================================
// Initialization
// =============================================================================

async function init() {
    updateClock();
    setInterval(updateClock, 1000);

    // Try to get server info first
    await fetchServerInfo();
    await checkServerStatus();

    if (state.token) {
        showBrowser();
        const savedPath = localStorage.getItem('entanglement_path') || '';
        await loadDirectory(savedPath);
    } else {
        showLogin();
    }

    setupEventListeners();
}

async function fetchServerInfo() {
    try {
        const response = await fetch(`${API_BASE}/server/info`);
        if (response.ok) {
            const data = await response.json();
            if (data.name) {
                state.serverName = data.name;
            }
        }
    } catch (e) {
        console.warn('Could not fetch server info, using default name');
    }
}

function setupEventListeners() {
    loginForm.addEventListener('submit', handleLogin);

    // User Dropdown Logic
    userMenuBtn.addEventListener('click', (e) => {
        e.stopPropagation();
        const expanded = userMenuBtn.getAttribute('aria-expanded') === 'true';
        userMenuBtn.setAttribute('aria-expanded', !expanded);
        userDropdown.hidden = expanded;
    });

    // Close dropdown when clicking outside
    document.addEventListener('click', (e) => {
        if (userMenuBtn && userDropdown && !userMenuBtn.contains(e.target) && !userDropdown.contains(e.target)) {
            userDropdown.hidden = true;
            userMenuBtn.setAttribute('aria-expanded', 'false');
        }
    });

    logoutBtn.addEventListener('click', handleLogout);

    // Theme toggle
    const themeToggle = document.getElementById('theme-toggle');
    if (themeToggle) {
        // Initialize toggle state
        const savedTheme = localStorage.getItem('entanglement_theme');
        const systemDark = window.matchMedia('(prefers-color-scheme: dark)').matches;

        if (savedTheme === 'dark' || (!savedTheme && systemDark)) {
            themeToggle.classList.add('active');
        }

        themeToggle.addEventListener('click', () => {
            // Determine current state
            const currentTheme = document.documentElement.getAttribute('data-theme');
            const isDark = currentTheme === 'dark' || (!currentTheme && systemDark);

            if (isDark) {
                document.documentElement.setAttribute('data-theme', 'light');
                localStorage.setItem('entanglement_theme', 'light');
                themeToggle.classList.remove('active');
            } else {
                document.documentElement.setAttribute('data-theme', 'dark');
                localStorage.setItem('entanglement_theme', 'dark');
                themeToggle.classList.add('active');
            }
        });
    }

    // Drag and drop upload
    setupDragAndDrop();

    // Context menu
    setupContextMenu();

    // Setup modal buttons (Conflicts, Sync Settings, Share)
    setupModals();
}

// =============================================================================
// Authentication
// =============================================================================

async function checkServerStatus() {
    try {
        const response = await fetch(`${API_BASE}/v1/files/list?path=`, {
            method: 'GET',
            headers: { 'Content-Type': 'application/json' },
        });
        // 401 means server is up but requires auth
        if (response.ok || response.status === 401) {
            statusIndicator.classList.add('connected');
            statusIndicator.classList.remove('error');
            serverStatus.textContent = 'Server connected';
            return true;
        }
    } catch (error) {
        statusIndicator.classList.add('error');
        statusIndicator.classList.remove('connected');
        serverStatus.textContent = 'Server unavailable';
    }
    return false;
}

async function handleLogin(e) {
    e.preventDefault();

    const username = document.getElementById('username').value;
    const password = document.getElementById('password').value;
    const submitBtn = loginForm.querySelector('button[type="submit"]');

    loginError.hidden = true;
    submitBtn.disabled = true;
    submitBtn.textContent = 'Authenticating...';

    try {
        const response = await fetch(`${API_BASE}/auth/login`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ username, password }),
        });

        if (!response.ok) {
            const data = await response.json().catch(() => ({}));
            throw new Error(data.message || 'Authentication failed');
        }

        const data = await response.json();

        state.token = data.token;
        state.username = data.username;
        state.isAdmin = data.is_admin;

        localStorage.setItem('entanglement_token', data.token);
        localStorage.setItem('entanglement_username', data.username);
        localStorage.setItem('entanglement_is_admin', data.is_admin ? 'true' : 'false');

        showBrowser();
        await loadDirectory('');

    } catch (error) {
        loginError.textContent = error.message;
        loginError.hidden = false;
    } finally {
        submitBtn.disabled = false;
        submitBtn.textContent = 'Sign In';
    }
}

function handleLogout() {
    // Disconnect WebSocket
    disconnectWebSocket();

    state.token = null;
    state.username = null;
    state.isAdmin = false;
    state.currentPath = '';
    state.serverName = 'Entanglement';

    localStorage.removeItem('entanglement_token');
    localStorage.removeItem('entanglement_username');
    localStorage.removeItem('entanglement_is_admin');

    showLogin();
}

// =============================================================================
// Views
// =============================================================================

function showLogin() {
    loginView.hidden = false;
    browserView.hidden = true;
    if (usersView) usersView.hidden = true;
    loginForm.reset();
    loginError.hidden = true;
}

function showBrowser() {
    loginView.hidden = true;
    browserView.hidden = false;
    if (usersView) usersView.hidden = true;
    if (userNameEl) userNameEl.textContent = state.username || 'User';
    // Connect to WebSocket for real-time sync
    connectWebSocket();
}

function showUsersView() {
    loginView.hidden = true;
    browserView.hidden = true;
    if (usersView) usersView.hidden = false;

    // Set user name in users view dropdown
    const usersUserName = document.getElementById('users-user-name');
    if (usersUserName) usersUserName.textContent = state.username || 'User';

    // Update time display
    updateUsersTime();

    // Load the users list
    loadAdminUsers();
}

function hideUsersView() {
    if (usersView) usersView.hidden = true;
    browserView.hidden = false;
}

function updateUsersTime() {
    const usersTime = document.getElementById('users-time');
    if (usersTime) {
        const now = new Date();
        usersTime.textContent = now.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
    }
}

// =============================================================================
// File Browser
// =============================================================================

async function loadDirectory(path) {
    state.currentPath = path;
    localStorage.setItem('entanglement_path', path);

    fileList.innerHTML = '';
    emptyState.hidden = true;
    loadingState.hidden = false;

    renderBreadcrumb(path);

    try {
        const response = await fetch(`${API_BASE}/v1/files/list?path=${encodeURIComponent(path)}`, {
            headers: {
                'Authorization': `Bearer ${state.token}`,
                'Content-Type': 'application/json',
            },
        });

        if (response.status === 401) {
            handleLogout();
            return;
        }

        if (!response.ok) {
            throw new Error('Failed to load directory');
        }

        const data = await response.json();

        loadingState.hidden = true;

        if (data.entries && data.entries.length > 0) {
            renderFileList(data.entries);
            itemCount.textContent = `${data.entries.length} item${data.entries.length !== 1 ? 's' : ''}`;
        } else {
            emptyState.hidden = false;
            itemCount.textContent = '0 items';
        }

    } catch (error) {
        console.error('Error loading directory:', error);
        loadingState.hidden = true;
        emptyState.querySelector('p').textContent = 'Error loading directory';
        emptyState.hidden = false;
    }
}

function renderBreadcrumb(path) {
    breadcrumb.innerHTML = '';

    // Root (Server Name)
    const root = document.createElement('span');
    root.className = 'breadcrumb-item' + (path === '' ? ' current' : '');
    root.textContent = state.serverName;
    root.onclick = () => loadDirectory('');
    breadcrumb.appendChild(root);

    if (path) {
        const parts = path.replace(/\/$/, '').split('/').filter(Boolean);
        let accumulated = '';

        parts.forEach((part, index) => {
            accumulated += part + '/';
            const isLast = index === parts.length - 1;

            const separator = document.createElement('span');
            separator.className = 'breadcrumb-separator';
            separator.textContent = '/';
            breadcrumb.appendChild(separator);

            const item = document.createElement('span');
            item.className = 'breadcrumb-item' + (isLast ? ' current' : '');
            item.textContent = part;
            if (!isLast) {
                const pathToLoad = accumulated;
                item.onclick = () => loadDirectory(pathToLoad);
            }
            breadcrumb.appendChild(item);
        });
    }

    // Auto-scroll breadcrumb to show current location (rightmost)
    requestAnimationFrame(() => {
        breadcrumb.scrollLeft = breadcrumb.scrollWidth;
    });
}

function renderFileList(entries) {
    // Sort: folders first, then alphabetically
    const sorted = entries.sort((a, b) => {
        if (a.is_folder !== b.is_folder) {
            return a.is_folder ? -1 : 1;
        }
        return a.name.localeCompare(b.name);
    });

    sorted.forEach(entry => {
        const tr = document.createElement('tr');

        // Store entry data for context menu
        tr.dataset.entryData = JSON.stringify(entry);

        // Right-click context menu for file/folder
        tr.addEventListener('contextmenu', (e) => {
            e.preventDefault();
            e.stopPropagation();
            const entryData = JSON.parse(tr.dataset.entryData);
            const menuItems = [
                {
                    icon: ICON_RENAME,
                    label: 'Rename',
                    action: () => renameItem(entryData)
                }
            ];

            // Add share option for files and folders
            menuItems.push({
                icon: ICON_SHARE,
                label: 'Share',
                action: () => showShareModal(entryData)
            });

            // Add Download As ZIP for folders
            if (entryData.is_folder) {
                menuItems.push({
                    icon: ICON_DOWNLOAD,
                    label: 'Download As ZIP',
                    action: () => downloadFolderAsZip(entryData)
                });
            }

            menuItems.push({ separator: true });
            menuItems.push({
                icon: ICON_DELETE,
                label: 'Delete',
                danger: true,
                action: () => deleteItem(entryData)
            });

            showContextMenu(e.clientX, e.clientY, menuItems);
        });

        // Name column
        const tdName = document.createElement('td');
        tdName.className = 'col-name';

        const fileEntry = document.createElement('div');
        fileEntry.className = 'file-entry';

        const icon = document.createElement('img');
        icon.className = 'file-icon';
        icon.src = getFileTypeIcon(entry.name, entry.is_folder);
        icon.alt = entry.is_folder ? 'Folder' : 'File';
        icon.draggable = false;

        const name = document.createElement('span');
        name.className = 'file-name' + (entry.is_folder ? ' folder' : '');
        name.textContent = entry.name;

        if (entry.is_folder) {
            name.onclick = () => loadDirectory(entry.path);
        } else if (isVideoFile(entry.name)) {
            name.classList.add('previewable');
            name.onclick = () => previewVideo(entry);
        } else if (isAudioFile(entry.name)) {
            name.classList.add('previewable');
            name.onclick = () => playAudio(entry);
        } else if (isPreviewable(entry.name)) {
            name.classList.add('previewable');
            name.onclick = () => previewFile(entry);
        }

        fileEntry.appendChild(icon);
        fileEntry.appendChild(name);
        tdName.appendChild(fileEntry);

        // Size column
        const tdSize = document.createElement('td');
        tdSize.className = 'col-size';
        tdSize.textContent = entry.is_folder ? '—' : formatBytes(entry.size_bytes);

        // Modified column
        const tdModified = document.createElement('td');
        tdModified.className = 'col-modified';
        tdModified.textContent = formatDate(entry.updated_at);

        // Actions column (Download button only for files)
        const tdActions = document.createElement('td');
        tdActions.className = 'col-actions';

        if (!entry.is_folder && entry.version_id) {
            const downloadBtn = document.createElement('button');
            downloadBtn.className = 'btn-download';
            downloadBtn.textContent = 'Download';
            downloadBtn.onclick = () => downloadFile(entry);
            tdActions.appendChild(downloadBtn);
        }

        tr.appendChild(tdName);
        tr.appendChild(tdSize);
        tr.appendChild(tdModified);
        tr.appendChild(tdActions);

        fileList.appendChild(tr);
    });
}

// =============================================================================
// File Download
// =============================================================================

async function downloadFile(entry) {
    try {
        const response = await fetch(`${API_BASE}/v1/files/${entry.version_id}/download`, {
            headers: {
                'Authorization': `Bearer ${state.token}`,
            },
        });

        if (!response.ok) {
            throw new Error('Download failed');
        }

        const blob = await response.blob();
        const url = window.URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.style.display = 'none';
        a.href = url;
        a.download = entry.name;
        document.body.appendChild(a);
        a.click();
        window.URL.revokeObjectURL(url);
        document.body.removeChild(a);

    } catch (error) {
        console.error('Download error:', error);
        alert('Download failed: ' + error.message);
    }
}

// Download a folder as a ZIP archive
async function downloadFolderAsZip(entry) {
    if (!entry.is_folder) return;

    // Show loading state
    const folderName = entry.name || 'folder';

    try {
        const response = await fetch(`${API_BASE}/v1/files/download-zip?path=${encodeURIComponent(entry.path)}`, {
            headers: {
                'Authorization': `Bearer ${state.token}`,
            },
        });

        if (!response.ok) {
            const data = await response.json().catch(() => ({}));
            throw new Error(data.error || 'Download failed');
        }

        const blob = await response.blob();
        const url = window.URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.style.display = 'none';
        a.href = url;
        a.download = `${folderName}.zip`;
        document.body.appendChild(a);
        a.click();
        window.URL.revokeObjectURL(url);
        document.body.removeChild(a);

    } catch (error) {
        console.error('ZIP download error:', error);
        alert('ZIP download failed: ' + error.message);
    }
}


// =============================================================================
// File Preview
// =============================================================================

const PREVIEWABLE_IMAGE_EXTENSIONS = ['jpg', 'jpeg', 'png', 'gif', 'webp', 'svg', 'bmp', 'ico'];
const PREVIEWABLE_VIDEO_EXTENSIONS = ['mp4', 'mov', 'webm', 'm4v', 'ogv'];
const PREVIEWABLE_TEXT_EXTENSIONS = [
    'txt', 'md', 'markdown', 'json', 'yaml', 'yml', 'xml', 'csv',
    'js', 'jsx', 'ts', 'tsx', 'mjs', 'cjs',
    'py', 'pyw', 'rb', 'php', 'java', 'c', 'h', 'cpp', 'hpp', 'go', 'rs', 'swift',
    'html', 'htm', 'css', 'scss', 'sass', 'less',
    'sh', 'bash', 'zsh', 'ps1', 'bat', 'cmd',
    'sql', 'conf', 'cfg', 'ini', 'env', 'toml', 'plist',
    'log', 'gitignore', 'dockerignore', 'editorconfig'
];

function isPreviewable(filename) {
    const ext = filename.split('.').pop().toLowerCase();
    const basename = filename.toLowerCase();

    // Check for dotfiles that are text
    if (basename.startsWith('.') && !basename.includes('.', 1)) {
        return 'text';
    }

    if (PREVIEWABLE_IMAGE_EXTENSIONS.includes(ext)) {
        return 'image';
    }
    if (PREVIEWABLE_VIDEO_EXTENSIONS.includes(ext)) {
        return 'video';
    }
    if (ext === 'pdf') {
        return 'pdf';
    }
    if (PREVIEWABLE_TEXT_EXTENSIONS.includes(ext)) {
        return 'text';
    }
    return null;
}

async function previewFile(entry) {
    const previewType = isPreviewable(entry.name);
    if (!previewType || !entry.version_id) return;

    // Find the file icon and show loading state
    const fileRows = fileList.querySelectorAll('tr');
    for (const row of fileRows) {
        const fileName = row.querySelector('.file-name');
        if (fileName && fileName.textContent === entry.name) {
            const icon = row.querySelector('.file-icon');
            if (icon) {
                currentLoadingIcon = { element: icon, originalSrc: icon.src };
                icon.src = 'icons/loading.svg';
                icon.classList.add('loading-spin');
            }
            break;
        }
    }

    try {
        const response = await fetch(`${API_BASE}/v1/files/${entry.version_id}/download`, {
            headers: {
                'Authorization': `Bearer ${state.token}`,
            },
        });

        if (!response.ok) {
            restoreLoadingIcon();
            throw new Error('Failed to load file');
        }

        if (previewType === 'image') {
            const blob = await response.blob();
            const url = window.URL.createObjectURL(blob);
            const img = document.createElement('img');
            img.src = url;
            img.alt = entry.name;
            img.onload = () => {
                restoreLoadingIcon();
                previewContent.innerHTML = '';
                previewContent.appendChild(img);
                previewModal.hidden = false;
                document.body.style.overflow = 'hidden';
            };
            img.onerror = () => {
                restoreLoadingIcon();
                previewContent.innerHTML = '<span class="preview-error">Failed to load image</span>';
                previewModal.hidden = false;
                document.body.style.overflow = 'hidden';
            };
        } else if (previewType === 'video') {
            const blob = await response.blob();
            const url = window.URL.createObjectURL(blob);
            const video = document.createElement('video');
            video.src = url;
            video.controls = true;
            video.autoplay = true;
            video.onloadeddata = () => {
                restoreLoadingIcon();
                previewContent.innerHTML = '';
                previewContent.appendChild(video);
                previewModal.hidden = false;
                document.body.style.overflow = 'hidden';
            };
            video.onerror = () => {
                restoreLoadingIcon();
                previewContent.innerHTML = '<span class="preview-error">Failed to load video</span>';
                previewModal.hidden = false;
                document.body.style.overflow = 'hidden';
            };
        } else if (previewType === 'pdf') {
            const blob = await response.blob();
            const url = window.URL.createObjectURL(blob);
            const iframe = document.createElement('iframe');
            iframe.src = url;
            iframe.className = 'pdf-preview';
            iframe.title = entry.name;
            // PDF iframes don't reliably fire onload, show immediately
            restoreLoadingIcon();
            previewContent.innerHTML = '';
            previewContent.appendChild(iframe);
            previewModal.hidden = false;
            document.body.style.overflow = 'hidden';
        } else if (previewType === 'text') {
            const text = await response.text();
            const pre = document.createElement('pre');
            pre.textContent = text;
            restoreLoadingIcon();
            previewContent.innerHTML = '';
            previewContent.appendChild(pre);
            previewModal.hidden = false;
            document.body.style.overflow = 'hidden';
        }

    } catch (error) {
        restoreLoadingIcon();
        console.error('Preview error:', error);
        // Security: Use textContent to prevent XSS from error messages
        const errorSpan = document.createElement('span');
        errorSpan.className = 'preview-error';
        errorSpan.textContent = `Preview failed: ${error.message}`;
        previewContent.innerHTML = '';
        previewContent.appendChild(errorSpan);
        previewModal.hidden = false;
        document.body.style.overflow = 'hidden';
    }
}

function closePreview() {
    previewModal.hidden = true;
    document.body.style.overflow = '';
    // Stop any playing video and release resources
    const video = previewContent.querySelector('video');
    if (video) {
        video.pause();
        video.removeAttribute('src');
        video.load();
    }
    previewContent.innerHTML = '';
    restoreLoadingIcon();
}

let currentLoadingIcon = null;

async function previewVideo(entry) {
    if (!entry.version_id) return;

    // Find the file icon and show loading state
    const fileRows = fileList.querySelectorAll('tr');
    for (const row of fileRows) {
        const fileName = row.querySelector('.file-name');
        if (fileName && fileName.textContent === entry.name) {
            const icon = row.querySelector('.file-icon');
            if (icon) {
                currentLoadingIcon = { element: icon, originalSrc: icon.src };
                icon.src = 'icons/loading.svg';
                icon.classList.add('loading-spin');
            }
            break;
        }
    }

    try {
        const response = await fetch(`${API_BASE}/v1/files/${entry.version_id}/download`, {
            headers: {
                'Authorization': `Bearer ${state.token}`,
            },
        });

        if (!response.ok) {
            restoreLoadingIcon();
            throw new Error('Failed to load video');
        }

        const blob = await response.blob();
        const url = window.URL.createObjectURL(blob);
        const video = document.createElement('video');
        video.src = url;
        video.controls = true;
        video.autoplay = true;

        // Wait for video to be ready before showing modal
        video.onloadeddata = () => {
            restoreLoadingIcon();
            previewContent.innerHTML = '';
            previewContent.appendChild(video);
            previewModal.hidden = false;
            document.body.style.overflow = 'hidden';
        };
        video.onerror = () => {
            restoreLoadingIcon();
            alert('Failed to load video');
        };

    } catch (error) {
        restoreLoadingIcon();
        console.error('Video preview error:', error);
        alert('Preview failed: ' + error.message);
    }
}

function restoreLoadingIcon() {
    if (currentLoadingIcon) {
        currentLoadingIcon.element.src = currentLoadingIcon.originalSrc;
        currentLoadingIcon.element.classList.remove('loading-spin');
        currentLoadingIcon = null;
    }
}

// Preview event listeners
if (previewClose) {
    previewClose.addEventListener('click', closePreview);
}
if (previewBackdrop) {
    previewBackdrop.addEventListener('click', closePreview);
}
document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && !previewModal.hidden) {
        closePreview();
    }
});

// =============================================================================
// Audio Player
// =============================================================================

const audioPlayer = document.getElementById('audio-player');
const audioPlayBtn = document.getElementById('audio-play-btn');
const audioPlayIcon = document.getElementById('audio-play-icon');
const audioTrackName = document.getElementById('audio-track-name');
const audioProgress = document.getElementById('audio-progress');
const audioCurrentTime = document.getElementById('audio-current-time');
const audioDuration = document.getElementById('audio-duration');
const audioCloseBtn = document.getElementById('audio-close-btn');

let audioElement = null;

// SVG Icons
const PLAY_ICON = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" fill="currentColor"><path d="M240,128a15.74,15.74,0,0,1-7.6,13.51L88.32,229.65a16,16,0,0,1-16.2.3A15.86,15.86,0,0,1,64,216.13V39.87a15.86,15.86,0,0,1,8.12-13.82,16,16,0,0,1,16.2.3L232.4,114.49A15.74,15.74,0,0,1,240,128Z"/></svg>';
const PAUSE_ICON = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" fill="currentColor"><path d="M216,48V208a16,16,0,0,1-16,16H160a16,16,0,0,1-16-16V48a16,16,0,0,1,16-16h40A16,16,0,0,1,216,48ZM96,32H56A16,16,0,0,0,40,48V208a16,16,0,0,0,16,16H96a16,16,0,0,0,16-16V48A16,16,0,0,0,96,32Z"/></svg>';

async function playAudio(entry) {
    if (!entry.version_id) return;

    // Stop any existing audio
    if (audioElement) {
        audioElement.pause();
        audioElement = null;
    }

    // Show player
    audioTrackName.textContent = entry.name;
    audioPlayer.hidden = false;
    audioPlayIcon.innerHTML = PLAY_ICON;
    audioProgress.value = 0;
    audioCurrentTime.textContent = '0:00';
    audioDuration.textContent = '0:00';
    updateProgressBackground(0);

    try {
        const response = await fetch(`${API_BASE}/v1/files/${entry.version_id}/download`, {
            headers: {
                'Authorization': `Bearer ${state.token}`,
            },
        });

        if (!response.ok) {
            throw new Error('Failed to load audio');
        }

        const blob = await response.blob();
        const url = window.URL.createObjectURL(blob);

        audioElement = new Audio(url);

        audioElement.addEventListener('loadedmetadata', () => {
            audioDuration.textContent = formatAudioTime(audioElement.duration);
        });

        audioElement.addEventListener('timeupdate', () => {
            if (audioElement.duration) {
                const percent = (audioElement.currentTime / audioElement.duration) * 100;
                audioProgress.value = percent;
                audioCurrentTime.textContent = formatAudioTime(audioElement.currentTime);
                updateProgressBackground(percent);
            }
        });

        audioElement.addEventListener('ended', () => {
            audioPlayIcon.innerHTML = PLAY_ICON;
        });

        audioElement.play();
        audioPlayIcon.innerHTML = PAUSE_ICON;

    } catch (error) {
        console.error('Audio error:', error);
        closeAudioPlayer();
    }
}

function toggleAudioPlayback() {
    if (!audioElement) return;

    if (audioElement.paused) {
        audioElement.play();
        audioPlayIcon.innerHTML = PAUSE_ICON;
    } else {
        audioElement.pause();
        audioPlayIcon.innerHTML = PLAY_ICON;
    }
}

function seekAudio(e) {
    if (!audioElement || !audioElement.duration) return;
    const percent = e.target.value;
    audioElement.currentTime = (percent / 100) * audioElement.duration;
    updateProgressBackground(percent);
}

function updateProgressBackground(percent) {
    const black = 'var(--color-text-primary)';
    const grey = 'var(--color-border)';
    audioProgress.style.background = `linear-gradient(to right, ${black} ${percent}%, ${grey} ${percent}%)`;
}

function closeAudioPlayer() {
    if (audioElement) {
        audioElement.pause();
        audioElement = null;
    }
    audioPlayer.hidden = true;
    // Reset progress bar background
    audioProgress.style.background = 'var(--color-border)';
}

function formatAudioTime(seconds) {
    if (!seconds || isNaN(seconds)) return '0:00';
    const mins = Math.floor(seconds / 60);
    const secs = Math.floor(seconds % 60);
    return `${mins}:${secs.toString().padStart(2, '0')}`;
}

// Audio player event listeners
if (audioPlayBtn) {
    audioPlayBtn.addEventListener('click', toggleAudioPlayback);
}
if (audioProgress) {
    audioProgress.addEventListener('input', seekAudio);
}
if (audioCloseBtn) {
    audioCloseBtn.addEventListener('click', closeAudioPlayer);
}

// =============================================================================
// Utilities
// =============================================================================

/**
 * Returns the icon path for a given file based on its extension
 */
function getFileTypeIcon(filename, isFolder) {
    if (isFolder) {
        return 'icons/folder.svg';
    }

    const ext = filename.split('.').pop().toLowerCase();

    // Image files
    if (['jpg', 'jpeg', 'png', 'gif', 'webp', 'svg', 'ico', 'bmp', 'tiff', 'heic', 'heif', 'raw', 'psd', 'ai'].includes(ext)) {
        return 'icons/image.svg';
    }

    // Video files
    if (['mp4', 'mov', 'avi', 'mkv', 'wmv', 'flv', 'webm', 'm4v', 'mpeg', 'mpg', '3gp'].includes(ext)) {
        return 'icons/video.svg';
    }

    // Audio files
    if (['mp3', 'wav', 'aac', 'flac', 'ogg', 'm4a', 'wma', 'aiff', 'aif', 'alac', 'opus'].includes(ext)) {
        return 'icons/audio.svg';
    }

    // Document files
    if (['pdf'].includes(ext)) {
        return 'icons/pdf.svg';
    }
    if (['doc', 'docx', 'odt', 'rtf'].includes(ext)) {
        return 'icons/document.svg';
    }
    if (['xls', 'xlsx', 'ods', 'csv'].includes(ext)) {
        return 'icons/spreadsheet.svg';
    }
    if (['ppt', 'pptx', 'odp', 'key'].includes(ext)) {
        return 'icons/presentation.svg';
    }
    if (['txt', 'md', 'markdown', 'rst'].includes(ext)) {
        return 'icons/text.svg';
    }

    // Code files
    if (['js', 'jsx', 'mjs', 'cjs'].includes(ext)) {
        return 'icons/code-js.svg';
    }
    if (['ts', 'tsx'].includes(ext)) {
        return 'icons/code-ts.svg';
    }
    if (['py', 'pyw', 'pyi'].includes(ext)) {
        return 'icons/code-python.svg';
    }
    if (['swift'].includes(ext)) {
        return 'icons/code-swift.svg';
    }
    if (['go'].includes(ext)) {
        return 'icons/code-go.svg';
    }
    if (['rs'].includes(ext)) {
        return 'icons/code-rust.svg';
    }
    if (['rb', 'erb'].includes(ext)) {
        return 'icons/code-ruby.svg';
    }
    if (['java', 'kt', 'kts', 'scala'].includes(ext)) {
        return 'icons/code-java.svg';
    }
    if (['c', 'h'].includes(ext)) {
        return 'icons/code-c.svg';
    }
    if (['cpp', 'cc', 'cxx', 'hpp', 'hxx'].includes(ext)) {
        return 'icons/code-cpp.svg';
    }
    if (['php'].includes(ext)) {
        return 'icons/code-php.svg';
    }
    if (['html', 'htm', 'xhtml'].includes(ext)) {
        return 'icons/code-html.svg';
    }
    if (['css', 'scss', 'sass', 'less'].includes(ext)) {
        return 'icons/code-css.svg';
    }

    // Config & Data
    if (['json', 'yaml', 'yml', 'toml', 'xml', 'plist', 'ini', 'conf', 'cfg'].includes(ext)) {
        return 'icons/config.svg';
    }
    if (['sh', 'bash', 'zsh', 'fish', 'ps1', 'bat', 'cmd'].includes(ext)) {
        return 'icons/terminal.svg';
    }
    if (['sql', 'db', 'sqlite', 'sqlite3'].includes(ext)) {
        return 'icons/database.svg';
    }

    // Archive files
    if (['zip', 'tar', 'gz', 'bz2', 'xz', '7z', 'rar', 'tgz', 'tbz2'].includes(ext)) {
        return 'icons/archive.svg';
    }

    // Executable / Binary / Packages
    if (['exe', 'app', 'msi', 'deb', 'rpm', 'apk', 'ipa', 'dmg', 'pkg'].includes(ext)) {
        return 'icons/package.svg';
    }

    // Font files
    if (['ttf', 'otf', 'woff', 'woff2', 'eot'].includes(ext)) {
        return 'icons/font.svg';
    }

    // 3D files
    if (['obj', 'fbx', 'blend', 'stl', '3ds', 'dae', 'gltf', 'glb'].includes(ext)) {
        return 'icons/3d.svg';
    }

    // Design files
    if (['sketch', 'fig', 'xd'].includes(ext)) {
        return 'icons/design.svg';
    }

    // eBooks
    if (['epub', 'mobi', 'azw', 'azw3'].includes(ext)) {
        return 'icons/ebook.svg';
    }

    // Lock / dotfiles
    if (['lock', 'lockb'].includes(ext) || filename.startsWith('.')) {
        return 'icons/lock.svg';
    }

    // Default file icon
    return 'icons/file.svg';
}

function formatBytes(bytes) {
    if (bytes === 0 || bytes === null || bytes === undefined) return '0 B';

    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    const k = 1024;
    const i = Math.floor(Math.log(bytes) / Math.log(k));

    return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + units[i];
}

function formatDate(isoString) {
    if (!isoString) return '—';

    const date = new Date(isoString);
    const year = date.getFullYear();
    const month = String(date.getMonth() + 1).padStart(2, '0');
    const day = String(date.getDate()).padStart(2, '0');
    const hours = String(date.getHours()).padStart(2, '0');
    const minutes = String(date.getMinutes()).padStart(2, '0');

    return `${year}-${month}-${day} ${hours}:${minutes}`;
}

function updateClock() {
    const now = new Date();
    // Use user's local timezone
    const timeString = now.toLocaleTimeString([], {
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
        hour12: true // Or true if preferred, keeping 24h as prev code implied or system default
    });

    // Get time zone abbreviation (e.g. EST, GMT, etc.)
    const timeZone = now.toLocaleTimeString([], { timeZoneName: 'short' }).split(' ').pop();

    const formattedTime = `${timeString} ${timeZone}`;

    // Update both file browser and user management clocks
    if (currentTime) currentTime.textContent = formattedTime;
    const usersTime = document.getElementById('users-time');
    if (usersTime) usersTime.textContent = formattedTime;
}

// =============================================================================
// Drag and Drop Upload
// =============================================================================

let dropOverlay = null;

function setupDragAndDrop() {
    // Prevent default drag behaviors on the whole window
    ['dragenter', 'dragover', 'dragleave', 'drop'].forEach(eventName => {
        document.body.addEventListener(eventName, preventDefaults, false);
    });

    // Show overlay on dragenter
    document.body.addEventListener('dragenter', showDropOverlay, false);
    document.body.addEventListener('dragover', showDropOverlay, false);
    document.body.addEventListener('dragleave', handleDragLeave, false);
    document.body.addEventListener('drop', handleDrop, false);
}

function preventDefaults(e) {
    e.preventDefault();
    e.stopPropagation();
}

function showDropOverlay(e) {
    if (!state.token) return; // Not logged in

    if (!dropOverlay) {
        dropOverlay = document.createElement('div');
        dropOverlay.className = 'drop-overlay';
        dropOverlay.innerHTML = `
            <div class="drop-overlay-content">
                <div class="drop-overlay-icon">
                    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" fill="currentColor" width="64" height="64">
                        <path d="M248,128a87.34,87.34,0,0,1-17.6,52.81,8,8,0,1,1-12.8-9.62A71.34,71.34,0,0,0,232,128a72,72,0,0,0-144,0,8,8,0,0,1-16,0,88,88,0,0,1,3.29-23.88C74.2,104,73.1,104,72,104a48,48,0,0,0,0,96H96a8,8,0,0,1,0,16H72A64,64,0,1,1,81.29,88.68,88,88,0,0,1,248,128Zm-90.34-5.66a8,8,0,0,0-11.32,0l-32,32a8,8,0,0,0,11.32,11.32L144,147.31V208a8,8,0,0,0,16,0V147.31l18.34,18.35a8,8,0,0,0,11.32-11.32Z"/>
                    </svg>
                </div>
                <div class="drop-overlay-text">Drop files to upload</div>
                <div class="drop-overlay-hint">Files will be uploaded to the current folder</div>
            </div>
        `;
        document.body.appendChild(dropOverlay);
    }
}

function handleDragLeave(e) {
    // Only hide if leaving the window entirely
    if (e.relatedTarget === null || !document.body.contains(e.relatedTarget)) {
        hideDropOverlay();
    }
}

function hideDropOverlay() {
    if (dropOverlay) {
        dropOverlay.remove();
        dropOverlay = null;
    }
}

async function handleDrop(e) {
    hideDropOverlay();

    if (!state.token) return;

    const files = e.dataTransfer.files;
    if (files.length === 0) return;

    // Upload each file
    for (const file of files) {
        await uploadFile(file);
    }

    // Refresh the directory
    await loadDirectory(state.currentPath);
}

async function uploadFile(file) {
    const progressEl = showUploadProgress(file.name);

    try {
        // Prepend leading slash if not present
        let filePath = state.currentPath ? `${state.currentPath}${file.name}` : file.name;
        if (!filePath.startsWith('/')) {
            filePath = '/' + filePath;
        }

        // Read file and convert to base64
        const arrayBuffer = await file.arrayBuffer();
        const bytes = new Uint8Array(arrayBuffer);
        let binary = '';
        for (let i = 0; i < bytes.byteLength; i++) {
            binary += String.fromCharCode(bytes[i]);
        }
        const base64Content = btoa(binary);

        const response = await fetch(`${API_BASE}/files`, {
            method: 'POST',
            headers: {
                'Authorization': `Bearer ${state.token}`,
                'Content-Type': 'application/json',
            },
            body: JSON.stringify({
                path: filePath,
                content: base64Content,
            }),
        });

        if (!response.ok) {
            const data = await response.json().catch(() => ({}));
            throw new Error(data.message || 'Upload failed');
        }

        console.log(`Uploaded: ${file.name}`);

    } catch (error) {
        console.error('Upload error:', error);
        alert(`Failed to upload ${file.name}: ${error.message}`);
    } finally {
        hideUploadProgress(progressEl);
    }
}

function showUploadProgress(filename) {
    const el = document.createElement('div');
    el.className = 'upload-progress';
    // Security: Build DOM elements programmatically to prevent XSS from filenames
    const spinner = document.createElement('div');
    spinner.className = 'upload-progress-spinner';
    const text = document.createElement('span');
    text.className = 'upload-progress-text';
    text.textContent = `Uploading ${filename}...`;
    el.appendChild(spinner);
    el.appendChild(text);
    document.body.appendChild(el);
    return el;
}

function hideUploadProgress(el) {
    if (el && el.parentNode) {
        el.remove();
    }
}

// =============================================================================
// New Folder Creation (UI removed but function kept for future use)
// =============================================================================

async function createNewFolder() {
    const folderName = prompt('Enter folder name:');
    if (!folderName || !folderName.trim()) return;

    const name = folderName.trim();

    // Validate folder name - prevent path traversal and invalid characters
    if (name.includes('/') || name.includes('\\') || name.includes('..') || name.includes('\0')) {
        alert('Invalid folder name. Cannot contain / \\ .. or null characters.');
        return;
    }

    // Build the full path
    const fullPath = state.currentPath ? `/${state.currentPath}${name}` : `/${name}`;

    try {
        const response = await fetch(`${API_BASE}/v1/files/directory`, {
            method: 'POST',
            headers: {
                'Authorization': `Bearer ${state.token}`,
                'Content-Type': 'application/json',
            },
            body: JSON.stringify({ path: fullPath }),
        });

        if (response.status === 401) {
            handleLogout();
            return;
        }

        if (!response.ok) {
            const data = await response.json().catch(() => ({}));
            throw new Error(data.message || 'Failed to create folder');
        }

        console.log(`Created folder: ${fullPath}`);
        await loadDirectory(state.currentPath);

    } catch (error) {
        console.error('Create folder error:', error);
        alert(`Failed to create folder: ${error.message}`);
    }
}

// =============================================================================
// File/Folder Deletion
// =============================================================================

async function deleteItem(entry) {
    const itemType = entry.is_folder ? 'folder' : 'file';
    const confirmed = confirm(`Are you sure you want to delete "${entry.name}"?\n\nThis action cannot be undone.`);
    if (!confirmed) return;

    console.log(`[Delete] Starting delete for ${itemType}: ${entry.name}, id: ${entry.id}`);

    try {
        const response = await fetch(`${API_BASE}/files/${entry.id}`, {
            method: 'DELETE',
            headers: {
                'Authorization': `Bearer ${state.token}`,
            },
        });

        console.log(`[Delete] Response status: ${response.status}`);

        if (response.status === 401) {
            handleLogout();
            return;
        }

        if (!response.ok) {
            const data = await response.json().catch(() => ({}));
            throw new Error(data.message || `Failed to delete ${itemType}`);
        }

        console.log(`[Delete] Successfully deleted ${itemType}: ${entry.name}`);
        console.log(`[Delete] Refreshing directory: ${state.currentPath}`);
        await loadDirectory(state.currentPath);
        console.log(`[Delete] Directory refresh complete`);

    } catch (error) {
        console.error('[Delete] Error:', error);
        alert(`Failed to delete: ${error.message}`);
    }
}

// =============================================================================
// File/Folder Rename
// =============================================================================

async function renameItem(entry) {
    const currentName = entry.name;
    const newName = prompt('Enter new name:', currentName);
    if (!newName || !newName.trim() || newName.trim() === currentName) return;

    const name = newName.trim();

    // Validate new name
    if (name.includes('/') || name.includes('\\') || name.includes('..') || name.includes('\0')) {
        alert('Invalid name. Cannot contain / \\ .. or null characters.');
        return;
    }

    // Build the new path
    const parentPath = state.currentPath ? `/${state.currentPath}` : '/';
    let newPath = parentPath.endsWith('/') ? parentPath + name : parentPath + '/' + name;

    // Preserve trailing slash for folders
    if (entry.is_folder && !newPath.endsWith('/')) {
        newPath += '/';
    }

    try {
        const response = await fetch(`${API_BASE}/files/${entry.id}`, {
            method: 'PATCH',
            headers: {
                'Authorization': `Bearer ${state.token}`,
                'Content-Type': 'application/json',
            },
            body: JSON.stringify({ path: newPath }),
        });

        if (response.status === 401) {
            handleLogout();
            return;
        }

        if (!response.ok) {
            const data = await response.json().catch(() => ({}));
            throw new Error(data.message || 'Failed to rename');
        }

        console.log(`Renamed: ${currentName} -> ${name}`);
        await loadDirectory(state.currentPath);

    } catch (error) {
        console.error('Rename error:', error);
        alert(`Failed to rename: ${error.message}`);
    }
}

// =============================================================================
// WebSocket Real-Time Sync
// =============================================================================

let ws = null;
let wsReconnectAttempts = 0;
const WS_MAX_RECONNECT_ATTEMPTS = 10;

function connectWebSocket() {
    if (!state.token) return;

    // Build WebSocket URL (same host, API port)
    const wsProtocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const wsUrl = `${wsProtocol}//${window.location.hostname}:1975/ws/sync?token=${state.token}`;

    console.log('[WebSocket] Connecting...');
    updateWsStatus('reconnecting');

    try {
        ws = new WebSocket(wsUrl);

        ws.onopen = () => {
            console.log('[WebSocket] Connected');
            wsReconnectAttempts = 0;
            updateWsStatus('connected');
        };

        ws.onmessage = (event) => {
            try {
                const notification = JSON.parse(event.data);
                console.log('[WebSocket] Received:', notification);

                if (notification.type === 'file_changed') {
                    // Check if this change affects the current directory
                    const changedPath = notification.path || '';
                    const changedDir = getParentDir(changedPath);
                    const currentDir = state.currentPath ? `/${state.currentPath}` : '/';

                    // Normalize for comparison
                    const normalizedChanged = changedDir.replace(/\/+$/, '') || '/';
                    const normalizedCurrent = currentDir.replace(/\/+$/, '') || '/';

                    if (normalizedChanged === normalizedCurrent ||
                        changedPath.startsWith(currentDir) ||
                        notification.action === 'delete') {
                        // Debounce rapid updates
                        if (window.wsRefreshTimeout) {
                            clearTimeout(window.wsRefreshTimeout);
                        }
                        window.wsRefreshTimeout = setTimeout(() => {
                            console.log('[WebSocket] Refreshing directory...');
                            loadDirectory(state.currentPath);
                        }, 500);
                    }
                }
            } catch (e) {
                console.warn('[WebSocket] Failed to parse message:', e);
            }
        };

        ws.onerror = (error) => {
            console.error('[WebSocket] Error:', error);
        };

        ws.onclose = (event) => {
            console.log('[WebSocket] Disconnected:', event.code, event.reason);
            updateWsStatus('disconnected');
            ws = null;

            // Attempt reconnection with exponential backoff
            if (state.token && wsReconnectAttempts < WS_MAX_RECONNECT_ATTEMPTS) {
                wsReconnectAttempts++;
                const delay = Math.min(Math.pow(2, wsReconnectAttempts - 1) * 1000, 60000);
                console.log(`[WebSocket] Reconnecting in ${delay}ms (attempt ${wsReconnectAttempts}/${WS_MAX_RECONNECT_ATTEMPTS})`);
                updateWsStatus('reconnecting');
                setTimeout(connectWebSocket, delay);
            }
        };
    } catch (error) {
        console.error('[WebSocket] Connection failed:', error);
        updateWsStatus('disconnected');
    }
}

function disconnectWebSocket() {
    if (ws) {
        ws.close();
        ws = null;
    }
    wsReconnectAttempts = WS_MAX_RECONNECT_ATTEMPTS; // Prevent reconnection
    updateWsStatus('disconnected');
}

function getParentDir(path) {
    if (!path || path === '/') return '/';
    const trimmed = path.replace(/\/+$/, '');
    const lastSlash = trimmed.lastIndexOf('/');
    if (lastSlash <= 0) return '/';
    return trimmed.substring(0, lastSlash + 1);
}

function updateWsStatus(status) {
    const wsStatus = document.getElementById('ws-status');
    if (!wsStatus) return;

    wsStatus.classList.remove('connected', 'disconnected', 'reconnecting');
    wsStatus.classList.add(status);

    const titles = {
        connected: 'Real-time sync: Connected',
        disconnected: 'Real-time sync: Disconnected',
        reconnecting: 'Real-time sync: Reconnecting...'
    };
    wsStatus.title = titles[status] || 'Real-time sync';
}

// =============================================================================
// Context Menu
// =============================================================================

let contextMenuTarget = null; // Stores the entry data when right-clicking on a file/folder

function showContextMenu(x, y, items) {
    // Build menu content
    contextMenu.innerHTML = '';

    items.forEach(item => {
        if (item.separator) {
            const sep = document.createElement('div');
            sep.className = 'context-menu-separator';
            contextMenu.appendChild(sep);
        } else {
            const btn = document.createElement('button');
            btn.className = 'context-menu-item' + (item.danger ? ' danger' : '');
            btn.innerHTML = item.icon + '<span>' + item.label + '</span>';
            btn.onclick = async (e) => {
                e.stopPropagation();
                hideContextMenu();
                await item.action();
            };
            contextMenu.appendChild(btn);
        }
    });

    // Position menu, ensuring it stays within viewport
    const menuWidth = 160;
    const menuHeight = items.length * 36 + 8; // Approximate height

    let posX = x;
    let posY = y;

    if (x + menuWidth > window.innerWidth) {
        posX = window.innerWidth - menuWidth - 8;
    }
    if (y + menuHeight > window.innerHeight) {
        posY = window.innerHeight - menuHeight - 8;
    }

    contextMenu.style.left = posX + 'px';
    contextMenu.style.top = posY + 'px';
    contextMenu.hidden = false;
}

function hideContextMenu() {
    contextMenu.hidden = true;
    contextMenuTarget = null;
}

function setupContextMenu() {
    // Close context menu when clicking elsewhere
    document.addEventListener('click', (e) => {
        if (!contextMenu.contains(e.target)) {
            hideContextMenu();
        }
    });

    // Close context menu on escape
    document.addEventListener('keydown', (e) => {
        if (e.key === 'Escape' && !contextMenu.hidden) {
            hideContextMenu();
        }
    });

    // Close context menu on scroll
    if (fileBrowser) {
        fileBrowser.addEventListener('scroll', hideContextMenu);
    }

    // Right-click on file browser area (blank space)
    if (fileBrowser) {
        fileBrowser.addEventListener('contextmenu', (e) => {
            // Only handle if clicking on blank area (not on a file row)
            const row = e.target.closest('tr');
            const isHeader = e.target.closest('thead');

            if (!row || isHeader) {
                e.preventDefault();
                showContextMenu(e.clientX, e.clientY, [
                    {
                        icon: ICON_NEW_FOLDER,
                        label: 'New Folder',
                        action: createNewFolder
                    }
                ]);
            }
        });
    }
}

function getEntryFromRow(row) {
    // Extract entry data from the row's data attributes
    if (row && row.dataset.entryData) {
        return JSON.parse(row.dataset.entryData);
    }
    return null;
}

// =============================================================================
// Conflicts Management
// =============================================================================

let currentShareFileId = null;

function setupModals() {
    console.log('[Modals] Setting up modals...');

    // Setup conflicts button
    const conflictsBtn = document.getElementById('conflicts-btn');
    const conflictsModal = document.getElementById('conflicts-modal');

    console.log('[Modals] conflictsBtn:', conflictsBtn);
    console.log('[Modals] conflictsModal:', conflictsModal);

    if (conflictsBtn && conflictsModal) {
        conflictsBtn.addEventListener('click', () => {
            console.log('[Modals] Conflicts button clicked!');
            if (userDropdown) userDropdown.hidden = true;
            loadConflicts();
            conflictsModal.hidden = false;
        });
        console.log('[Modals] Conflicts button listener attached');
    } else {
        console.error('[Modals] Could not find conflicts button or modal');
    }

    // Setup sync settings button
    const syncSettingsBtn = document.getElementById('sync-settings-btn');
    const syncSettingsModal = document.getElementById('sync-settings-modal');

    console.log('[Modals] syncSettingsBtn:', syncSettingsBtn);
    console.log('[Modals] syncSettingsModal:', syncSettingsModal);

    if (syncSettingsBtn && syncSettingsModal) {
        syncSettingsBtn.addEventListener('click', () => {
            console.log('[Modals] Sync Settings button clicked!');
            if (userDropdown) userDropdown.hidden = true;
            loadSyncSettings();
            syncSettingsModal.hidden = false;
        });
        console.log('[Modals] Sync Settings button listener attached');
    } else {
        console.error('[Modals] Could not find sync settings button or modal');
    }

    // Setup admin button (only visible if admin)
    const adminBtn = document.getElementById('admin-btn');
    if (adminBtn) {
        adminBtn.addEventListener('click', () => {
            if (userDropdown) userDropdown.hidden = true;
            showUsersView();
        });
    }

    // Setup users view back button (Entanglement breadcrumb)
    const usersBackRoot = document.getElementById('users-back-root');
    if (usersBackRoot) {
        usersBackRoot.addEventListener('click', () => {
            hideUsersView();
        });
    }

    // Setup users view dropdown
    const usersMenuBtn = document.getElementById('users-menu-btn');
    const usersDropdown = document.getElementById('users-dropdown');
    if (usersMenuBtn && usersDropdown) {
        usersMenuBtn.addEventListener('click', (e) => {
            e.stopPropagation();
            const expanded = usersMenuBtn.getAttribute('aria-expanded') === 'true';
            usersMenuBtn.setAttribute('aria-expanded', !expanded);
            usersDropdown.hidden = expanded;
        });

        // Close dropdown when clicking outside
        document.addEventListener('click', (e) => {
            if (!usersMenuBtn.contains(e.target) && !usersDropdown.contains(e.target)) {
                usersDropdown.hidden = true;
                usersMenuBtn.setAttribute('aria-expanded', 'false');
            }
        });
    }

    // Setup users view theme toggle
    const usersThemeToggle = document.getElementById('users-theme-toggle');
    if (usersThemeToggle) {
        // Initialize toggle state
        const savedTheme = localStorage.getItem('entanglement_theme');
        if (savedTheme === 'dark') {
            usersThemeToggle.classList.add('active');
        }

        usersThemeToggle.addEventListener('click', () => {
            const isDark = document.documentElement.getAttribute('data-theme') === 'dark';
            const themeToggle = document.getElementById('theme-toggle');
            if (isDark) {
                document.documentElement.removeAttribute('data-theme');
                localStorage.setItem('entanglement_theme', 'light');
                usersThemeToggle.classList.remove('active');
                if (themeToggle) themeToggle.classList.remove('active');
            } else {
                document.documentElement.setAttribute('data-theme', 'dark');
                localStorage.setItem('entanglement_theme', 'dark');
                usersThemeToggle.classList.add('active');
                if (themeToggle) themeToggle.classList.add('active');
            }
        });
    }

    // Setup users view logout button
    const usersLogoutBtn = document.getElementById('users-logout-btn');
    if (usersLogoutBtn) {
        usersLogoutBtn.addEventListener('click', handleLogout);
    }

    // Create user modal
    const createUserBtn = document.getElementById('create-user-btn');
    const createUserModal = document.getElementById('create-user-modal');
    const createUserForm = document.getElementById('create-user-form');
    const cancelCreateUser = document.getElementById('cancel-create-user');

    if (createUserBtn && createUserModal) {
        createUserBtn.addEventListener('click', () => {
            document.getElementById('create-user-error').hidden = true;
            createUserForm.reset();
            createUserModal.hidden = false;
            document.getElementById('new-username').focus();
        });
    }

    if (cancelCreateUser) {
        cancelCreateUser.addEventListener('click', () => {
            createUserModal.hidden = true;
        });
    }

    if (createUserForm) {
        createUserForm.addEventListener('submit', handleCreateUser);
    }

    // Reset password modal
    const resetPasswordForm = document.getElementById('reset-password-form');
    const cancelResetPassword = document.getElementById('cancel-reset-password');
    const resetPasswordModal = document.getElementById('reset-password-modal');

    if (cancelResetPassword) {
        cancelResetPassword.addEventListener('click', () => {
            resetPasswordModal.hidden = true;
        });
    }

    if (resetPasswordForm) {
        resetPasswordForm.addEventListener('submit', handleResetPassword);
    }

    // Setup share button
    const createShareBtn = document.getElementById('create-share-btn');
    if (createShareBtn) {
        createShareBtn.addEventListener('click', createShareLink);
    }

    // Setup copy share link button
    const copyShareLinkBtn = document.getElementById('copy-share-link');
    if (copyShareLinkBtn) {
        copyShareLinkBtn.addEventListener('click', () => {
            const linkInput = document.getElementById('share-link');
            if (linkInput) {
                linkInput.select();
                document.execCommand('copy');
                copyShareLinkBtn.textContent = 'Copied!';
                setTimeout(() => {
                    copyShareLinkBtn.textContent = 'Copy';
                }, 2000);
            }
        });
    }


    // Setup save public URL button
    const savePublicUrlBtn = document.getElementById('save-public-url-btn');
    if (savePublicUrlBtn) {
        savePublicUrlBtn.addEventListener('click', savePublicUrl);
    }

    // Setup cancel settings button
    const cancelSettingsBtn = document.getElementById('cancel-settings');
    if (cancelSettingsBtn) {
        cancelSettingsBtn.addEventListener('click', () => {
            document.getElementById('sync-settings-modal').hidden = true;
        });
    }

    // Close modal handlers
    document.querySelectorAll('.modal-close').forEach(btn => {
        btn.addEventListener('click', (e) => {
            const modal = e.target.closest('.modal');
            if (modal) modal.hidden = true;
        });
    });

    document.querySelectorAll('.modal-backdrop').forEach(backdrop => {
        backdrop.addEventListener('click', (e) => {
            const modal = e.target.closest('.modal');
            if (modal) modal.hidden = true;
        });
    });

    // Setup custom selects
    setupCustomSelects();
}

// =============================================================================
// Custom Select Component
// =============================================================================

function setupCustomSelects() {
    document.querySelectorAll('.custom-select').forEach(select => {
        const trigger = select.querySelector('.custom-select-trigger');
        const dropdown = select.querySelector('.custom-select-dropdown');
        const valueDisplay = select.querySelector('.custom-select-value');
        const hiddenInput = select.parentElement.querySelector('input[type="hidden"]');
        const options = select.querySelectorAll('.custom-select-option');

        if (!trigger || !dropdown) return;

        // Toggle dropdown on trigger click
        trigger.addEventListener('click', (e) => {
            e.stopPropagation();
            const isOpen = !dropdown.hidden;

            // Close all other custom selects first
            document.querySelectorAll('.custom-select').forEach(s => {
                s.classList.remove('open');
                const d = s.querySelector('.custom-select-dropdown');
                if (d) d.hidden = true;
            });

            // Toggle this one
            if (!isOpen) {
                select.classList.add('open');
                dropdown.hidden = false;
            }
        });

        // Handle option selection
        options.forEach(option => {
            option.addEventListener('click', (e) => {
                e.stopPropagation();
                const value = option.dataset.value;
                const text = option.textContent;

                // Update display
                if (valueDisplay) valueDisplay.textContent = text;

                // Update hidden input
                if (hiddenInput) hiddenInput.value = value;

                // Update selected state
                options.forEach(o => o.classList.remove('selected'));
                option.classList.add('selected');

                // Close dropdown
                select.classList.remove('open');
                dropdown.hidden = true;
            });
        });

        // Set initial selected state
        const initialValue = hiddenInput?.value || '';
        options.forEach(option => {
            if (option.dataset.value === initialValue) {
                option.classList.add('selected');
            }
        });
    });

    // Close dropdowns when clicking outside
    document.addEventListener('click', () => {
        document.querySelectorAll('.custom-select').forEach(select => {
            select.classList.remove('open');
            const dropdown = select.querySelector('.custom-select-dropdown');
            if (dropdown) dropdown.hidden = true;
        });
    });
}

async function loadConflicts() {
    const conflictsList = document.getElementById('conflicts-list');
    const noConflicts = document.getElementById('no-conflicts');

    if (!conflictsList) return;

    conflictsList.innerHTML = '<p>Loading conflicts...</p>';

    try {
        const response = await fetch(`${API_BASE}/conflicts`, {
            headers: {
                'Authorization': `Bearer ${state.token}`,
            },
        });

        if (!response.ok) throw new Error('Failed to load conflicts');

        const data = await response.json();

        if (data.conflicts.length === 0) {
            conflictsList.innerHTML = '';
            if (noConflicts) noConflicts.hidden = false;
        } else {
            if (noConflicts) noConflicts.hidden = true;
            conflictsList.innerHTML = data.conflicts.map(conflict => `
                <div class="conflict-item" data-id="${conflict.id}">
                    <div class="conflict-info">
                        <span class="conflict-path">${escapeHtml(conflict.file_path)}</span>
                        <span class="conflict-type">${conflict.conflict_type}</span>
                        <span class="conflict-date">${formatDate(conflict.detected_at)}</span>
                    </div>
                    <div class="conflict-actions">
                        <button class="btn-small" onclick="resolveConflict('${conflict.id}', 'keep_local')">Keep Local</button>
                        <button class="btn-small" onclick="resolveConflict('${conflict.id}', 'keep_remote')">Keep Remote</button>
                        <button class="btn-small btn-secondary" onclick="resolveConflict('${conflict.id}', 'keep_both')">Keep Both</button>
                    </div>
                </div>
            `).join('');
        }
    } catch (error) {
        console.error('Error loading conflicts:', error);
        conflictsList.innerHTML = '<p class="error">Failed to load conflicts</p>';
    }
}

async function resolveConflict(conflictId, resolution) {
    try {
        const response = await fetch(`${API_BASE}/conflicts/${conflictId}/resolve`, {
            method: 'POST',
            headers: {
                'Authorization': `Bearer ${state.token}`,
                'Content-Type': 'application/json',
            },
            body: JSON.stringify({ resolution }),
        });

        if (!response.ok) throw new Error('Failed to resolve conflict');

        // Reload conflicts list
        loadConflicts();

    } catch (error) {
        console.error('Error resolving conflict:', error);
        alert('Failed to resolve conflict: ' + error.message);
    }
}

// =============================================================================
// File Sharing
// =============================================================================

function showShareModal(entry) {
    const shareModal = document.getElementById('share-modal');
    const shareForm = document.getElementById('share-form');
    const shareResult = document.getElementById('share-result');

    if (!shareModal) return;

    currentShareFileId = entry.id;
    if (shareForm) shareForm.hidden = false;
    if (shareResult) shareResult.hidden = true;

    // Reset form
    const expiryEl = document.getElementById('share-expiry');
    const passwordEl = document.getElementById('share-password');
    const maxDownloadsEl = document.getElementById('share-max-downloads');

    if (expiryEl) expiryEl.value = '';
    if (passwordEl) passwordEl.value = '';
    if (maxDownloadsEl) maxDownloadsEl.value = '';

    // Reset custom select display
    const expirySelect = document.getElementById('share-expiry-select');
    if (expirySelect) {
        const valueDisplay = expirySelect.querySelector('.custom-select-value');
        if (valueDisplay) valueDisplay.textContent = 'Never';
        expirySelect.querySelectorAll('.custom-select-option').forEach(opt => {
            opt.classList.toggle('selected', opt.dataset.value === '');
        });
    }

    shareModal.hidden = false;
}

async function createShareLink() {
    if (!currentShareFileId) return;

    const createShareBtn = document.getElementById('create-share-btn');
    const shareForm = document.getElementById('share-form');
    const shareResult = document.getElementById('share-result');

    const expiryHours = document.getElementById('share-expiry')?.value;
    const password = document.getElementById('share-password')?.value;
    const maxDownloads = document.getElementById('share-max-downloads')?.value;

    if (createShareBtn) {
        createShareBtn.disabled = true;
        createShareBtn.textContent = 'Creating...';
    }

    try {
        const body = {
            file_id: currentShareFileId,
        };

        if (expiryHours) body.expires_in_hours = parseInt(expiryHours);
        if (password) body.password = password;
        if (maxDownloads) body.max_downloads = parseInt(maxDownloads);

        const response = await fetch(`${API_BASE}/shares`, {
            method: 'POST',
            headers: {
                'Authorization': `Bearer ${state.token}`,
                'Content-Type': 'application/json',
            },
            body: JSON.stringify(body),
        });

        if (!response.ok) throw new Error('Failed to create share link');

        const data = await response.json();

        // Show result
        if (shareForm) shareForm.hidden = true;
        if (shareResult) shareResult.hidden = false;

        // Use custom PUBLIC_URL if set, otherwise use the server-provided URL
        const customPublicUrl = localStorage.getItem('entanglement_public_url');
        let shareUrl = data.share_url;
        if (customPublicUrl && data.token) {
            shareUrl = `${customPublicUrl}/share.html#${data.token}`;
        }

        const shareLinkInput = document.getElementById('share-link');
        if (shareLinkInput) shareLinkInput.value = shareUrl;

    } catch (error) {
        console.error('Error creating share:', error);
        alert('Failed to create share link: ' + error.message);
    } finally {
        if (createShareBtn) {
            createShareBtn.disabled = false;
            createShareBtn.textContent = 'Create Share Link';
        }
    }
}

// =============================================================================
// Settings
// =============================================================================

async function loadSyncSettings() {
    const devicesList = document.getElementById('devices-list');
    const publicUrlInput = document.getElementById('public-url-setting');
    const publicUrlHint = document.getElementById('public-url-hint');

    // Load public URL setting from localStorage
    if (publicUrlInput) {
        const savedPublicUrl = localStorage.getItem('entanglement_public_url') || '';
        publicUrlInput.value = savedPublicUrl;
        updatePublicUrlHint(publicUrlHint, savedPublicUrl);
    }

    // Load devices
    try {
        const devicesResponse = await fetch(`${API_BASE}/sync/devices`, {
            headers: {
                'Authorization': `Bearer ${state.token}`,
            },
        });

        if (devicesResponse.ok) {
            const devices = await devicesResponse.json();
            renderDevices(devices);
        }
    } catch (error) {
        console.error('Error loading devices:', error);
        if (devicesList) devicesList.innerHTML = '<p class="error">Failed to load devices</p>';
    }
}

function renderDevices(devices) {
    const devicesList = document.getElementById('devices-list');
    if (!devicesList) return;

    if (devices.length === 0) {
        devicesList.innerHTML = '<p class="empty-hint">No devices registered.</p>';
        return;
    }

    devicesList.innerHTML = devices.map(device => `
        <div class="device-item">
            <div class="device-info">
                <span class="device-name">${escapeHtml(device.device_name || device.device_id)}</span>
                <span class="device-last-seen">Last seen: ${formatDate(device.last_seen_at)}</span>
            </div>
            <span class="device-status ${device.is_active ? 'active' : 'inactive'}">${device.is_active ? 'Active' : 'Inactive'}</span>
        </div>
    `).join('');
}

// =============================================================================
// Public URL Setting
// =============================================================================

function updatePublicUrlHint(hintEl, url) {
    if (!hintEl) return;
    if (url) {
        hintEl.textContent = `Share links will use: ${url}/share.html#TOKEN`;
    } else {
        hintEl.textContent = `Share links will use: ${window.location.origin}/share.html#TOKEN`;
    }
}

function savePublicUrl() {
    const publicUrlInput = document.getElementById('public-url-setting');
    const publicUrlHint = document.getElementById('public-url-hint');
    const saveBtn = document.getElementById('save-public-url-btn');

    if (!publicUrlInput) return;

    let url = publicUrlInput.value.trim();

    // Remove trailing slash if present
    if (url.endsWith('/')) {
        url = url.slice(0, -1);
    }

    // Validate URL if provided
    if (url && !url.match(/^https?:\/\/.+/)) {
        alert('Please enter a valid URL starting with http:// or https://');
        return;
    }

    // Save to localStorage
    if (url) {
        localStorage.setItem('entanglement_public_url', url);
    } else {
        localStorage.removeItem('entanglement_public_url');
    }

    // Update hint
    updatePublicUrlHint(publicUrlHint, url);

    // Close the modal
    document.getElementById('sync-settings-modal').hidden = true;
}

function getPublicUrl() {
    return localStorage.getItem('entanglement_public_url') || window.location.origin;
}

// =============================================================================
// Utility: HTML Escape
// =============================================================================

function escapeHtml(text) {
    if (!text) return '';
    const div = document.createElement('div');
    div.textContent = text;
    return div.innerHTML;
}

// =============================================================================
// Forgot Password
// =============================================================================

const forgotPasswordLink = document.getElementById('forgot-password-link');

if (forgotPasswordLink) {
    forgotPasswordLink.addEventListener('click', async (e) => {
        e.preventDefault();

        const email = document.getElementById('email').value.trim();
        if (!email) {
            loginError.textContent = 'Please enter your email address first';
            loginError.hidden = false;
            return;
        }

        forgotPasswordLink.textContent = 'Sending...';
        forgotPasswordLink.style.pointerEvents = 'none';

        try {
            const response = await fetch(`${API_BASE}/auth/forgot-password`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ email }),
            });

            // Always show success to prevent email enumeration
            loginError.style.backgroundColor = 'rgba(52, 199, 89, 0.1)';
            loginError.style.color = '#34c759';
            loginError.textContent = 'If an account exists, a reset link has been sent.';
            loginError.hidden = false;

            // In debug mode, the server returns the token
            if (response.ok) {
                const data = await response.json();
                if (data.debug_token) {
                    console.log('[Debug] Reset token:', data.debug_token);
                }
            }

        } catch (error) {
            // Still show success to prevent enumeration
            loginError.style.backgroundColor = 'rgba(52, 199, 89, 0.1)';
            loginError.style.color = '#34c759';
            loginError.textContent = 'If an account exists, a reset link has been sent.';
            loginError.hidden = false;
        } finally {
            forgotPasswordLink.textContent = 'Forgot password?';
            forgotPasswordLink.style.pointerEvents = 'auto';

            // Reset error style after 5 seconds
            setTimeout(() => {
                loginError.style.backgroundColor = '';
                loginError.style.color = '';
                loginError.hidden = true;
            }, 5000);
        }
    });
}

// =============================================================================
// Email Verification
// =============================================================================

const settingsBadge = document.getElementById('settings-badge');
const verifyEmailBtn = document.getElementById('verify-email-btn');

// Check email verification status and update UI
async function checkEmailVerificationStatus() {
    if (!state.token) return;

    try {
        // Try to get user info (we'll use a simple check - the server doesn't have a dedicated endpoint yet)
        // For now, we'll show the verification option always and let the server handle it
        // This could be enhanced to check actual status via a /auth/me endpoint

        // Show verification option - in a real implementation, check email_verified status
        // For now, always show the option so users can resend verification if needed
        if (settingsBadge) settingsBadge.hidden = false;
        if (verifyEmailBtn) verifyEmailBtn.hidden = false;

    } catch (error) {
        console.warn('Could not check verification status:', error);
    }
}

if (verifyEmailBtn) {
    verifyEmailBtn.addEventListener('click', async () => {
        verifyEmailBtn.textContent = 'Sending...';
        verifyEmailBtn.disabled = true;

        try {
            const response = await fetch(`${API_BASE}/auth/send-verification`, {
                method: 'POST',
                headers: {
                    'Authorization': `Bearer ${state.token}`,
                    'Content-Type': 'application/json',
                },
            });

            if (response.ok) {
                const data = await response.json();
                verifyEmailBtn.textContent = 'Email Sent!';
                verifyEmailBtn.style.color = '#34c759';

                // In debug mode, log the token
                if (data.debug_token) {
                    console.log('[Debug] Verification token:', data.debug_token);
                }

                // Hide badge after successful send
                if (settingsBadge) settingsBadge.hidden = true;

                setTimeout(() => {
                    verifyEmailBtn.textContent = 'Verify Email';
                    verifyEmailBtn.style.color = '';
                    verifyEmailBtn.hidden = true;
                }, 3000);
            } else {
                const data = await response.json().catch(() => ({}));
                if (data.message && data.message.includes('already verified')) {
                    verifyEmailBtn.textContent = 'Already Verified ✓';
                    verifyEmailBtn.style.color = '#34c759';
                    if (settingsBadge) settingsBadge.hidden = true;
                } else {
                    verifyEmailBtn.textContent = 'Failed';
                    verifyEmailBtn.style.color = '#ff3b30';
                }
                setTimeout(() => {
                    verifyEmailBtn.textContent = 'Verify Email';
                    verifyEmailBtn.style.color = '';
                }, 2000);
            }

        } catch (error) {
            console.error('Verification error:', error);
            verifyEmailBtn.textContent = 'Failed';
            setTimeout(() => {
                verifyEmailBtn.textContent = 'Verify Email';
            }, 2000);
        } finally {
            verifyEmailBtn.disabled = false;
        }
    });
}

// Patch showBrowser to check verification status
const originalShowBrowser = showBrowser;
showBrowser = function () {
    originalShowBrowser();
    // Show admin button if user is admin
    const adminBtn = document.getElementById('admin-btn');
    if (adminBtn && state.isAdmin) {
        adminBtn.hidden = false;
    }
};

// =============================================================================
// Admin Management
// =============================================================================

async function loadAdminUsers() {
    const userList = document.getElementById('users-list');
    const errorEl = document.getElementById('users-error');
    const usersCount = document.getElementById('users-count');
    errorEl.hidden = true;

    try {
        const response = await fetch(`${API_BASE}/admin/users`, {
            headers: { 'Authorization': `Bearer ${state.token}` }
        });

        if (!response.ok) {
            const data = await response.json().catch(() => ({}));
            throw new Error(data.message || 'Failed to load users');
        }

        const users = await response.json();
        renderAdminUsers(users);
        if (usersCount) {
            usersCount.textContent = `${users.length} user${users.length !== 1 ? 's' : ''}`;
        }
    } catch (error) {
        errorEl.textContent = error.message;
        errorEl.hidden = false;
        userList.innerHTML = '<tr><td colspan="4" class="empty-state">Failed to load users</td></tr>';
    }
}

function renderAdminUsers(users) {
    const userList = document.getElementById('users-list');

    if (users.length === 0) {
        userList.innerHTML = '<tr><td colspan="4" class="empty-state">No users found</td></tr>';
        return;
    }

    userList.innerHTML = users.map(user => {
        const role = user.is_admin ? 'Admin' : 'User';
        const roleClass = user.is_admin ? 'role-admin' : 'role-user';
        const created = new Date(user.created_at).toLocaleDateString();

        // Actions: Reset Password and Delete
        // Styled as .btn-toolbar for consistency with "Add User"
        return `
            <tr>
                <td class="col-name">
                    <div class="user-entry">
                        <span class="user-name">${escapeHtml(user.username)}</span>
                    </div>
                </td>
                <td class="col-role"><span class="role-badge ${roleClass}">${role}</span></td>
                <td class="col-created">${created}</td>
                <td class="col-actions">
                    <div class="action-cell">
                        <button class="btn-toolbar btn-action-reset" onclick="showResetPasswordModal('${user.id}', '${escapeHtml(user.username)}')">
                            <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="11" width="18" height="11" rx="2" ry="2"></rect><path d="M7 11V7a5 5 0 0 1 10 0v4"></path></svg>
                            <span class="btn-text">Reset Password</span>
                        </button>
                        <button class="btn-toolbar btn-action-delete" onclick="deleteUser('${user.id}', '${escapeHtml(user.username)}')">
                            <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"></polyline><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"></path><line x1="10" y1="11" x2="10" y2="17"></line><line x1="14" y1="11" x2="14" y2="17"></line></svg>
                            <span class="btn-text">Delete</span>
                        </button>
                    </div>
                </td>
            </tr>
        `;
    }).join('');
}

async function handleCreateUser(e) {
    e.preventDefault();
    const errorEl = document.getElementById('create-user-error');
    errorEl.hidden = true;

    const username = document.getElementById('new-username').value;
    const password = document.getElementById('new-password').value;
    const isAdmin = document.getElementById('new-is-admin').checked;

    try {
        const response = await fetch(`${API_BASE}/admin/users`, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
                'Authorization': `Bearer ${state.token}`
            },
            body: JSON.stringify({ username, password, is_admin: isAdmin })
        });

        if (!response.ok) {
            const data = await response.json().catch(() => ({}));
            throw new Error(data.message || 'Failed to create user');
        }

        document.getElementById('create-user-modal').hidden = true;
        loadAdminUsers();
    } catch (error) {
        errorEl.textContent = error.message;
        errorEl.hidden = false;
    }
}

async function deleteUser(userId, username) {
    if (!confirm(`Delete user "${username}"? This cannot be undone.`)) return;

    const errorEl = document.getElementById('users-error');
    errorEl.hidden = true;

    try {
        const response = await fetch(`${API_BASE}/admin/users/${userId}`, {
            method: 'DELETE',
            headers: { 'Authorization': `Bearer ${state.token}` }
        });

        if (!response.ok) {
            const data = await response.json().catch(() => ({}));
            throw new Error(data.message || 'Failed to delete user');
        }

        loadAdminUsers();
    } catch (error) {
        errorEl.textContent = error.message;
        errorEl.hidden = false;
    }
}

function showResetPasswordModal(userId, username) {
    document.getElementById('reset-user-id').value = userId;
    document.getElementById('reset-username-display').textContent = username;
    document.getElementById('reset-new-password').value = '';
    document.getElementById('reset-password-error').hidden = true;
    document.getElementById('reset-password-modal').hidden = false;
    document.getElementById('reset-new-password').focus();
}

async function handleResetPassword(e) {
    e.preventDefault();
    const errorEl = document.getElementById('reset-password-error');
    errorEl.hidden = true;

    const userId = document.getElementById('reset-user-id').value;
    const newPassword = document.getElementById('reset-new-password').value;

    try {
        const response = await fetch(`${API_BASE}/admin/users/${userId}/password`, {
            method: 'PUT',
            headers: {
                'Content-Type': 'application/json',
                'Authorization': `Bearer ${state.token}`
            },
            body: JSON.stringify({ new_password: newPassword })
        });

        if (!response.ok) {
            const data = await response.json().catch(() => ({}));
            throw new Error(data.message || 'Failed to reset password');
        }

        document.getElementById('reset-password-modal').hidden = true;
        alert('Password updated successfully');
    } catch (error) {
        errorEl.textContent = error.message;
        errorEl.hidden = false;
    }
}

function escapeHtml(str) {
    const div = document.createElement('div');
    div.textContent = str;
    return div.innerHTML;
}

// =============================================================================
// Start
// =============================================================================

init();
