local helpers = {}

function helpers.trim(value)
  return value:match("^%s*(.-)%s*$")
end

function helpers.join.left(first, second)
  return first .. ":" .. second
end

function helpers:describe()
  return "helpers"
end

return helpers
