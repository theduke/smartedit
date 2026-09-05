class Release
  DRAFT = "draft"

  def label
    DRAFT
  end

  # TODO: remove before release
  def compatibility_alias
    label
  end
end
