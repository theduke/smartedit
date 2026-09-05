const plugin = {};

plugin.install = function install(target) {
  target.enabled = true;
};

plugin.run = (value) => value.toUpperCase();

plugin.Widget = function Widget(name) {
  this.name = name;
};

plugin.Widget.prototype.render = function render() {
  return this.name;
};

module.exports = plugin;
