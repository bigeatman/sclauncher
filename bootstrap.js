/* ES5 runtime patch for the verified GameLobbyPanel Preact bundle. */
(function (window, document) {
    "use strict";
    var VERSION = "20261005", MARKER = "__scLobbyRuntimeUiV1";
    var API_NAMES = ["Open", "Closed", "Computer"];
    var SLOT_STATES = ["open", "closed", "computer", "human"];
    var attempts = 0, previous = window.__scLobbyRuntimeUi;
    if (!window.location || !/(^|\/)GameLobbyPanel\.html$/i.test(window.location.pathname || "")) { return; }
    if (previous && previous.version === VERSION && previous.running) { return; }
    var status = { version: VERSION, running: true, status: "WAITING", error: "" };
    window.__scLobbyRuntimeUi = status;
    function publish(value, error) {
        status.status = value; status.error = error || ""; status.running = value === "WAITING";
        var element = document.documentElement;
        if (element && typeof element.setAttribute === "function") {
            element.setAttribute("data-sc-lobby-ui-version", VERSION);
            element.setAttribute("data-sc-lobby-ui", error ? "ERROR:" + String(error).substring(0, 160) : value);
        }
    }
    function sameId(left, right) {
        return left !== null && left !== undefined && right !== null && right !== undefined && String(left) === String(right);
    }
    function findById(items, id) {
        if (!Array.isArray(items)) { return null; }
        for (var i = 0; i < items.length; i += 1) {
            if (items[i] && sameId(items[i].id, id)) { return items[i]; }
        }
        return null;
    }
    function normalSlot(state, id) {
        var composition = state && state.teamComposition, teams = composition && composition.teams;
        if (!Array.isArray(teams) || !findById(state.players, id)) { return false; }
        for (var i = 0; i < teams.length; i += 1) {
            if (teams[i] && findById(teams[i].slots, id)) { return true; }
        }
        return false;
    }
    function slotOptions(component) {
        var options = [];
        for (var i = 0; i < API_NAMES.length; i += 1) {
            // Existing method retains the bundle's localized P array closure.
            var label = component.getDisplayedName("", API_NAMES[i].toLowerCase());
            if (typeof label !== "string" || label.length === 0) { return null; }
            options.push({ apiName: API_NAMES[i], name: label, id: i });
        }
        return options;
    }
    function replaceSlotOptions(existing, replacements) {
        var output = [], inserted = false;
        existing = Array.isArray(existing) ? existing : [];
        for (var i = 0; i < existing.length; i += 1) {
            var option = existing[i];
            if (option && API_NAMES.indexOf(option.apiName) !== -1) {
                if (!inserted) { output = output.concat(replacements); inserted = true; }
            } else { output.push(option); }
        }
        return inserted ? output : output.concat(replacements);
    }
    function currentOption(row, requested) {
        var options = row && row.dropdownOptions;
        if (!Array.isArray(options)) { return null; }
        for (var i = 0; i < options.length; i += 1) {
            var option = options[i];
            if (!option) { continue; }
            if (requested.apiName) {
                if (option.apiName === requested.apiName) { return option; }
            } else if (!option.apiName && sameId(option.id, requested.id) && option.name === requested.name) { return option; }
        }
        return null;
    }
    function isMovement(state, row, option) {
        var selectedName = row.name && row.name.name;
        if (typeof option.name !== "string" || option.name === selectedName || sameId(option.id, row.id)) { return false; }
        return normalSlot(state, option.id) || !!findById(state.observers, option.id);
    }
    function isLobbyComponent(component) {
        return component && typeof component.generateDisplayTable === "function" &&
            typeof component.onUserNameDropdownWidgetChange === "function" &&
            typeof component.getDisplayedName === "function" && typeof component.sendMessageWrapper === "function" &&
            component.state && Array.isArray(component.state.players) && Array.isArray(component.state.observers);
    }
    function install(component) {
        var prototype = Object.getPrototypeOf(component);
        while (prototype && !Object.prototype.hasOwnProperty.call(prototype, "generateDisplayTable")) { prototype = Object.getPrototypeOf(prototype); }
        // Verified bundle methods are prototype methods. Do not patch a foreign object.
        if (!prototype || !Object.prototype.hasOwnProperty.call(prototype, "onUserNameDropdownWidgetChange")) { return false; }
        if (!prototype[MARKER]) {
            var originalTable = prototype.generateDisplayTable, originalChange = prototype.onUserNameDropdownWidgetChange;
            prototype.generateDisplayTable = function () {
                var state = this.state;
                if (!state || !state.localPlayer) { return {}; }
                var table = originalTable.apply(this, arguments), local = state.localPlayer, info = state.gameInfo;
                if (!table || !local.host || !info || info.replay || info.savedGame || state.lockAllOptions) { return table; }
                var options = slotOptions(this);
                if (!options) { return table; }
                for (var i = 0; i < state.players.length; i += 1) {
                    var player = state.players[i];
                    if (!player || sameId(local.id, player.id) || !normalSlot(state, player.id) || SLOT_STATES.indexOf(player.state) === -1) { continue; }
                    var row = table[player.id];
                    if (!row) { continue; }
                    row.dropdownOptions = replaceSlotOptions(row.dropdownOptions, options);
                    row.hideDropdown = false;
                }
                return table;
            };
            prototype.onUserNameDropdownWidgetChange = function (oldRow, dropdownName, requested) {
                var state = this.state;
                if (!oldRow || !requested || !state || !state.localPlayer || !state.localPlayer.host || state.lockAllOptions) { return; }
                var table = this.generateDisplayTable(), row = table && table[oldRow.id];
                if (!row || row.hideDropdown) { return; }
                var option = currentOption(row, requested);
                if (!option) { return; }
                if (API_NAMES.indexOf(option.apiName) !== -1 || option.apiName === "Ban Player") {
                    if (sameId(state.localPlayer.id, row.id)) { return; }
                } else if (!option.apiName && !isMovement(state, row, option)) { return; }
                // Preserve original closure, transport and native HostRequest authority.
                return originalChange.call(this, row, dropdownName, option);
            };
            Object.defineProperty(prototype, MARKER, { value: VERSION, configurable: false });
        }
        if (typeof component.forceUpdate === "function") { component.forceUpdate(); }
        return true;
    }
    function findComponent() {
        var root = document.getElementById("root");
        if (!root) { return null; }
        var nodes = [root], descendants = root.getElementsByTagName ? root.getElementsByTagName("*") : [];
        var queue = [], seen = [], i;
        for (i = 0; i < descendants.length && i < 4096; i += 1) { nodes.push(descendants[i]); }
        for (i = 0; i < nodes.length; i += 1) {
            // Actual bundle: Preact 8 DOM base._component and __u parent chain.
            if (nodes[i]._component) { queue.push(nodes[i]._component); }
            var keys = Object.getOwnPropertyNames(nodes[i]);
            for (var j = 0; j < keys.length; j += 1) {
                if (keys[j].indexOf("__reactInternalInstance$") === 0) { queue.push(nodes[i][keys[j]]); }
            }
        }
        for (i = 0; i < queue.length && i < 512; i += 1) {
            var value = queue[i];
            if (!value || typeof value !== "object" || seen.indexOf(value) !== -1) { continue; }
            seen.push(value);
            if (isLobbyComponent(value)) { return value; }
            if (value._instance) { queue.push(value._instance); }
            if (value.__u) { queue.push(value.__u); }
            if (value._component) { queue.push(value._component); }
            if (value._renderedComponent) { queue.push(value._renderedComponent); }
            if (value._renderedChildren) {
                var childKeys = Object.keys(value._renderedChildren);
                for (var c = 0; c < childKeys.length; c += 1) { queue.push(value._renderedChildren[childKeys[c]]); }
            }
            if (value._currentElement && value._currentElement._owner) { queue.push(value._currentElement._owner); }
        }
        return null;
    }
    function attempt() {
        try { var component = findComponent(); if (component && install(component)) { publish("READY"); return; } }
        catch (error) { publish("ERROR", String(error && error.message || error)); return; }
        attempts += 1;
        if (attempts >= 60) { publish("ERROR", "GameLobbyPanel component not found within 15 seconds"); return; }
        window.setTimeout(attempt, 250);
    }
    publish("WAITING"); attempt();
}(window, document));
